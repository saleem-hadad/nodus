//! The pool of Claude accounts the server's sessions run on (SRV-100…105).
//!
//! - **Accounts** live in `claude-accounts.json` (0600), each with its
//!   `claude setup-token` token, which never goes back out: members see
//!   [`ClaudeAccount`]. A new token is tried with one `claude -p` run, in
//!   the session host as `nodus-run`, before it's taken (SRV-100).
//! - **Picking** (SRV-101): every Claude Code process takes the `ok` account
//!   least used in its five hours (unknown counts as 0), then the one used
//!   least recently.
//! - **Usage** (SRV-102): each session's status line reports its account's
//!   `rate_limits`, and only with the session's own token (SRV-143).
//! - **Limits** (SRV-103): a turn that fails with `rate_limit` makes its
//!   account `limited` until its window resets (an hour when that's not
//!   known); `authentication_failed`, `account_on_hold` and
//!   `billing_error` make it `failed`, and every member hears of it.
//! - **Moving** (SRV-104, SRV-105): the session whose turn hit the limit is
//!   stopped and resumed on another account with `Continue.`; with none
//!   free it waits, and resumes once one is. An account that fails, or is
//!   removed, has all its live sessions moved the same way. The sessions
//!   waiting are kept in `claude-waiting.json`, so a restart of `serve`
//!   still resumes them.
//! - **Doubt** (SRV-106): what a session reports comes from a process that
//!   repo content or a web page may steer, and each account it moves to
//!   hands it that account's token. So a session gets an account it hasn't
//!   run on at most once in 30 minutes; until then it goes back only to
//!   one it has, or waits. And no window is taken to reset more than a
//!   week ahead.
//!
//! What a session reports arrives while the session host's messages wait,
//! so the pool only takes note there; a task of its own ([`run`]) moves
//! sessions, frees accounts when their windows reset, and tells members.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nodus_core::{Nodus, PoolAccount};
use nodus_domain::Event;
use nodus_wire::{AccountState, AuditMember, ClaudeAccount, UsageWindow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::auth::{Auth, Who};
use crate::store::{self, now_ms, random_hex};

/// Where the pool is kept, beside the rest of the server's data (SRV-003).
pub(crate) const ACCOUNTS_FILE: &str = "claude-accounts.json";
/// Where the sessions waiting for an account are kept (SRV-104).
pub(crate) const WAITING_FILE: &str = "claude-waiting.json";
/// How long an account is `limited` when its window's reset isn't known.
const LIMIT_UNKNOWN_MS: f64 = 60.0 * 60.0 * 1000.0;
/// The furthest ahead a window resets: Claude's longest is a week (SRV-106).
const RESET_AHEAD_MS: f64 = 7.0 * 24.0 * 60.0 * 60.0 * 1000.0;
/// How often a session gets an account it hasn't run on (SRV-106).
const NEW_ACCOUNT_EVERY_MS: f64 = 30.0 * 60.0 * 1000.0;
/// What a session's start fails with while it may only go back to the
/// accounts it has run on and none of those is free (SRV-106).
pub(crate) const NO_ACCOUNT_YET: &str = "This session moved to another Claude account less than 30 minutes ago, and the ones it has run on are at their limit. It can go on once one is free, or in half an hour.";
/// The longest label.
const LABEL_CHARS: usize = 64;
/// The longest token taken: `claude setup-token`'s are about a hundred.
const TOKEN_CHARS: usize = 4096;
/// What a start fails with while the pool is empty (SRV-101).
pub(crate) const NO_ACCOUNTS: &str = "No Claude accounts yet. Add one in Settings → Servers.";
/// The `error`s of a failed turn that mean the account itself stopped
/// working (SRV-103).
const FAILED_ERRORS: [&str; 3] = ["authentication_failed", "account_on_hold", "billing_error"];

/// One account, as the file keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    id: String,
    label: String,
    #[serde(default)]
    added_by: Option<AuditMember>,
    token: String,
    added_at: f64,
    #[serde(default)]
    state: AccountState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resets_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seven_day: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

impl Stored {
    /// What members see of it: everything but the token.
    fn public(&self, sessions: u32) -> ClaudeAccount {
        ClaudeAccount {
            id: self.id.clone(),
            label: self.label.clone(),
            added_by: self.added_by.clone(),
            added_at: self.added_at,
            state: self.state,
            resets_at: self.resets_at,
            five_hour: self.five_hour,
            seven_day: self.seven_day,
            last_used_at: self.last_used_at,
            failure: self.failure.clone(),
            sessions,
        }
    }

    /// Its five hours' use for picking: unknown, or a window that has reset
    /// since, counts as 0 (SRV-101).
    fn five_hour_use(&self, now: f64) -> f64 {
        self.five_hour
            .filter(|window| window.resets_at.is_none_or(|at| at > now))
            .map_or(0.0, |window| window.used_percentage)
    }

    /// Until when a `rate_limit` keeps it out (SRV-103): the week's reset
    /// when the week is used up, else the five hours', else an hour.
    fn limited_until(&self, now: f64) -> f64 {
        let reset = |window: Option<UsageWindow>| window.and_then(|w| w.resets_at).filter(|&at| at > now);
        let week_full = self.seven_day.is_some_and(|w| w.used_percentage >= 100.0);
        let week = week_full.then(|| reset(self.seven_day)).flatten();
        week.or_else(|| reset(self.five_hour))
            .unwrap_or(now + LIMIT_UNKNOWN_MS)
            .min(now + RESET_AHEAD_MS)
    }
}

/// What the pool's task does, in order.
enum Work {
    /// The accounts changed: every member's app hears of it.
    Changed,
    /// A session's turn hit its account's limit: move it, or let it wait.
    Limited { session_id: String },
    /// An account was removed: its sessions move.
    Removed { account_id: String },
    /// An account failed (SRV-103): its live sessions move, the one whose
    /// turn it failed told to carry on.
    Failed { account_id: String, session_id: String },
    /// An account may be free now: waiting sessions go to it.
    Freed,
}

/// The pool.
pub(crate) struct Pool {
    file: PathBuf,
    auth: Arc<Auth>,
    accounts: Mutex<Vec<Stored>>,
    work: mpsc::UnboundedSender<Work>,
    /// Taken by [`run`] once the core is up.
    todo: Mutex<Option<mpsc::UnboundedReceiver<Work>>>,
    /// Sessions waiting for an account to free up, oldest first (SRV-104),
    /// as `claude-waiting.json` keeps them.
    waiting: Mutex<Vec<Waiting>>,
    waiting_file: PathBuf,
    /// The accounts each session has run on (SRV-106).
    seen: Mutex<HashMap<String, Seen>>,
}

/// The accounts a session has run on, and when it last got one it hadn't
/// (SRV-106). Its first account doesn't count.
#[derive(Debug, Default)]
struct Seen {
    accounts: HashSet<String>,
    new_at: Option<f64>,
}

impl Seen {
    /// The accounts it may take now: any once the half hour since its last
    /// new one has passed, else only those it has run on.
    fn limited_to(&self, now: f64) -> Option<&HashSet<String>> {
        self.new_at
            .is_some_and(|at| now - at < NEW_ACCOUNT_EVERY_MS)
            .then_some(&self.accounts)
    }
}

/// A session waiting for an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Waiting {
    session_id: String,
    /// The pool stopped it itself (a removed or failed account's), so it
    /// starts again even though it isn't live.
    #[serde(default)]
    stopped: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pool {
    /// Reads `claude-accounts.json` in `data`. One that's there but can't be
    /// read stops the server, rather than starting empty and writing over it.
    pub(crate) fn open(data: &Path, auth: Arc<Auth>) -> Result<Self, String> {
        let file = data.join(ACCOUNTS_FILE);
        let accounts: Vec<Stored> = store::read_json(&file)?;
        // Who waits is worth keeping, not worth stopping the server over.
        let waiting_file = data.join(WAITING_FILE);
        let waiting: Vec<Waiting> = store::read_json(&waiting_file).unwrap_or_else(|err| {
            tracing_err(&err);
            Vec::new()
        });
        let (work, todo) = mpsc::unbounded_channel();
        Ok(Self {
            file,
            auth,
            accounts: Mutex::new(accounts),
            work,
            todo: Mutex::new(Some(todo)),
            waiting: Mutex::new(waiting),
            waiting_file,
            seen: Mutex::default(),
        })
    }

    /// Changes who waits, and keeps it when it changed.
    fn change_waiting(&self, change: impl FnOnce(&mut Vec<Waiting>)) {
        let mut waiting = lock(&self.waiting);
        let before = waiting.clone();
        change(&mut waiting);
        if *waiting != before
            && let Err(err) = store::write_json(&self.waiting_file, &*waiting)
        {
            tracing_err(&err);
        }
    }

    fn save(&self, accounts: &[Stored]) {
        if let Err(err) = store::write_json(&self.file, &accounts) {
            tracing_err(&err);
        }
    }

    fn changed(&self) {
        let _ = self.work.send(Work::Changed);
    }

    // ---- what members do (SRV-100, SRV-105)

    /// `accounts:list`, with how many live sessions each runs.
    pub(crate) fn list(&self, nodus: &Nodus) -> Vec<ClaudeAccount> {
        self.free_due();
        let accounts = lock(&self.accounts).clone();
        accounts
            .iter()
            .map(|account| {
                let sessions = nodus.sessions_on_account(&account.id).len();
                account.public(u32::try_from(sessions).unwrap_or(u32::MAX))
            })
            .collect()
    }

    /// `accounts:add` (SRV-100): the token is tried first, through the
    /// session host, and taken only if Claude answers with it.
    pub(crate) async fn add(
        &self,
        nodus: &Nodus,
        who: &Who,
        label: &str,
        token: &str,
    ) -> Result<ClaudeAccount, String> {
        let label = check_label(label)?;
        let token = token.trim();
        if token.is_empty() {
            return Err("Paste the token claude setup-token printed".to_owned());
        }
        if token.chars().count() > TOKEN_CHARS || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("That isn't a token claude setup-token prints".to_owned());
        }
        if let Some(known) = lock(&self.accounts).iter().find(|a| a.token == token) {
            return Err(format!("That token is in the pool already, as {}", known.label));
        }
        nodus.check_claude_token(token).await?;
        let (added_by, _) = self.auth.actor(who);
        let account = Stored {
            id: random_hex(8),
            label: label.clone(),
            added_by,
            token: token.to_owned(),
            added_at: now_ms(),
            state: AccountState::Ok,
            resets_at: None,
            five_hour: None,
            seven_day: None,
            last_used_at: None,
            failure: None,
        };
        {
            let mut accounts = lock(&self.accounts);
            if accounts.iter().any(|a| a.token == token) {
                return Err(format!("That token is in the pool already, as {label}"));
            }
            accounts.push(account.clone());
            self.save(&accounts);
        }
        self.auth.record(
            who,
            "account.added",
            json!({ "id": account.id, "label": account.label }),
        );
        self.changed();
        let _ = self.work.send(Work::Freed);
        Ok(account.public(0))
    }

    /// `accounts:rename`: the cards running on it say the new label too.
    pub(crate) fn rename(&self, nodus: &Nodus, who: &Who, id: &str, label: &str) -> Result<ClaudeAccount, String> {
        let label = check_label(label)?;
        let (renamed, was) = {
            let mut accounts = lock(&self.accounts);
            let account = accounts.iter_mut().find(|a| a.id == id).ok_or(NO_SUCH_ACCOUNT)?;
            let was = std::mem::replace(&mut account.label, label.clone());
            let renamed = account.clone();
            self.save(&accounts);
            (renamed, was)
        };
        if was != label {
            self.auth
                .record(who, "account.renamed", json!({ "id": id, "label": label, "was": was }));
            nodus.relabel_account(id, &label);
        }
        self.changed();
        Ok(renamed.public(u32::try_from(nodus.sessions_on_account(id).len()).unwrap_or(u32::MAX)))
    }

    /// `accounts:remove` (SRV-105): the sessions running on it move to
    /// another. The token itself works until its owner revokes it.
    pub(crate) fn remove(&self, who: &Who, id: &str) -> Result<(), String> {
        let removed = {
            let mut accounts = lock(&self.accounts);
            let at = accounts.iter().position(|a| a.id == id).ok_or(NO_SUCH_ACCOUNT)?;
            let removed = accounts.remove(at);
            self.save(&accounts);
            removed
        };
        self.auth.record(
            who,
            "account.removed",
            json!({ "id": removed.id, "label": removed.label }),
        );
        self.changed();
        let _ = self.work.send(Work::Removed { account_id: removed.id });
        Ok(())
    }

    /// SRV-040: a removed member's accounts, when the remover asked.
    pub(crate) fn remove_added_by(&self, who: &Who, github_id: u64) {
        let ids: Vec<String> = lock(&self.accounts)
            .iter()
            .filter(|a| a.added_by.as_ref().is_some_and(|by| by.github_id == github_id))
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            let _ = self.remove(who, &id);
        }
    }

    // ---- what sessions do (SRV-101…103)

    /// SRV-103: a limited account whose window has reset is `ok` again.
    /// Whether any was.
    fn free_due(&self) -> bool {
        let now = now_ms();
        let mut accounts = lock(&self.accounts);
        let mut freed = false;
        for account in accounts.iter_mut() {
            if account.state == AccountState::Limited && account.resets_at.is_none_or(|at| at <= now) {
                account.state = AccountState::Ok;
                account.resets_at = None;
                freed = true;
            }
        }
        if freed {
            self.save(&accounts);
            drop(accounts);
            self.changed();
            let _ = self.work.send(Work::Freed);
        }
        freed
    }

    /// When the next limited account frees up.
    fn next_reset(&self) -> Option<f64> {
        lock(&self.accounts)
            .iter()
            .filter(|a| a.state == AccountState::Limited)
            .filter_map(|a| a.resets_at)
            .reduce(f64::min)
    }

    /// Whether some account is `ok` now.
    fn has_free(&self) -> bool {
        self.free_due();
        lock(&self.accounts).iter().any(|a| a.state == AccountState::Ok)
    }

    /// SRV-101: the account a Claude Code process takes, marked used: a
    /// session's (`session`), or a background run's. A session that got an
    /// account it hadn't run on in the last half hour takes only one it has
    /// (SRV-106).
    pub(crate) fn pick(&self, session: Option<&str>) -> Result<PoolAccount, String> {
        self.free_due();
        let now = now_ms();
        let picked = {
            let mut accounts = lock(&self.accounts);
            if accounts.is_empty() {
                return Err(NO_ACCOUNTS.to_owned());
            }
            let mut seen = lock(&self.seen);
            let limited_to = session.and_then(|id| seen.get(id)).and_then(|s| s.limited_to(now));
            let best = accounts
                .iter()
                .enumerate()
                .filter(|(_, a)| a.state == AccountState::Ok)
                .filter(|(_, a)| limited_to.is_none_or(|ids| ids.contains(&a.id)))
                .min_by(|(_, a), (_, b)| {
                    a.five_hour_use(now)
                        .total_cmp(&b.five_hour_use(now))
                        .then(a.last_used_at.unwrap_or(0.0).total_cmp(&b.last_used_at.unwrap_or(0.0)))
                })
                .map(|(at, _)| at);
            let Some(at) = best else {
                let any_free = accounts.iter().any(|a| a.state == AccountState::Ok);
                return Err(if limited_to.is_some() && any_free {
                    NO_ACCOUNT_YET.to_owned()
                } else {
                    none_free(&accounts, now)
                });
            };
            if let Some(id) = session {
                let seen = seen.entry(id.to_owned()).or_default();
                if seen.accounts.insert(accounts[at].id.clone()) && seen.accounts.len() > 1 {
                    seen.new_at = Some(now);
                }
            }
            drop(seen);
            // Strictly after every other use, so picks in the same
            // millisecond still go round the accounts in turn.
            let latest = accounts.iter().filter_map(|a| a.last_used_at).fold(0.0, f64::max);
            let account = &mut accounts[at];
            account.last_used_at = Some(now.max(latest + 1.0));
            let picked = PoolAccount {
                id: account.id.clone(),
                label: account.label.clone(),
                token: account.token.clone(),
            };
            self.save(&accounts);
            picked
        };
        self.changed();
        Ok(picked)
    }

    /// What a session sent with its own token, on the account its card
    /// records (SRV-102, SRV-103).
    pub(crate) fn session_hook(&self, session_id: &str, account: Option<&str>, event: &str, payload: &Value) {
        let Some(account) = account else { return };
        match event {
            "StatusLine" => self.usage(account, payload),
            "StopFailure" => {
                let error = payload.get("error").and_then(Value::as_str).unwrap_or("");
                if error == "rate_limit" {
                    self.limit(account);
                    let _ = self.work.send(Work::Limited {
                        session_id: session_id.to_owned(),
                    });
                } else if FAILED_ERRORS.contains(&error) {
                    self.fail(account, error);
                    let _ = self.work.send(Work::Failed {
                        account_id: account.to_owned(),
                        session_id: session_id.to_owned(),
                    });
                }
            }
            _ => {}
        }
    }

    /// SRV-102: `rate_limits` from a status line, on its account.
    fn usage(&self, id: &str, payload: &Value) {
        let Some(limits) = payload.get("rate_limits").filter(|l| l.is_object()) else {
            return;
        };
        let now = now_ms();
        let five_hour = window(limits.get("five_hour"), now);
        let seven_day = window(limits.get("seven_day"), now);
        if five_hour.is_none() && seven_day.is_none() {
            return;
        }
        let mut accounts = lock(&self.accounts);
        let Some(account) = accounts.iter_mut().find(|a| a.id == id) else {
            return;
        };
        let (was_five, was_seven) = (account.five_hour, account.seven_day);
        account.five_hour = five_hour.or(account.five_hour);
        account.seven_day = seven_day.or(account.seven_day);
        if (account.five_hour, account.seven_day) != (was_five, was_seven) {
            self.save(&accounts);
            drop(accounts);
            self.changed();
        }
    }

    /// SRV-103: `rate_limit`: out until its window resets.
    fn limit(&self, id: &str) {
        let now = now_ms();
        let mut accounts = lock(&self.accounts);
        let Some(account) = accounts.iter_mut().find(|a| a.id == id) else {
            return;
        };
        if account.state == AccountState::Failed {
            return;
        }
        account.state = AccountState::Limited;
        account.resets_at = Some(account.limited_until(now));
        self.save(&accounts);
        drop(accounts);
        self.changed();
    }

    /// SRV-103: Claude turned the account down. Recorded, and every member
    /// is told, once.
    fn fail(&self, id: &str, error: &str) {
        let label = {
            let mut accounts = lock(&self.accounts);
            let Some(account) = accounts.iter_mut().find(|a| a.id == id) else {
                return;
            };
            if account.state == AccountState::Failed {
                return;
            }
            account.state = AccountState::Failed;
            account.resets_at = None;
            account.failure = Some(error.to_owned());
            let label = account.label.clone();
            self.save(&accounts);
            label
        };
        self.auth.audit.record(
            "account.failed",
            None,
            None,
            json!({ "id": id, "label": label, "error": error }),
        );
        self.auth.notify(
            nodus_wire::ACCOUNT_FAILED,
            format!("{label}'s Claude account stopped working."),
        );
        self.changed();
    }

    // ---- the task

    /// Takes a session's turn that hit the limit (SRV-104): to another
    /// account when one is free, else it waits, saying until when.
    async fn move_session(&self, nodus: &Nodus, session_id: &str) {
        // One stopped since isn't started again unasked.
        if !nodus.is_live(session_id) {
            return;
        }
        if !self.has_free() {
            self.wait(nodus, session_id, false);
            return;
        }
        if !self.resume_on_another(nodus, session_id, true).await {
            self.wait(nodus, session_id, false);
        }
    }

    /// Stops the session and resumes it, which takes an account; `busy`
    /// sends `Continue.` too. The card says where it went. False when it
    /// couldn't start.
    async fn resume_on_another(&self, nodus: &Nodus, session_id: &str, busy: bool) -> bool {
        let Some(project_id) = nodus.board().session_project(session_id) else {
            return true;
        };
        let was = nodus.account_of(session_id);
        match nodus.restart_session(&project_id, session_id, busy).await {
            Ok(()) => {
                let now = nodus.account_label_of(session_id).unwrap_or_default();
                let note = if was.is_some() && nodus.account_of(session_id) == was {
                    format!("Resumed on {now}")
                } else {
                    format!("Moved to {now}")
                };
                nodus.note_account(session_id, Some(&note));
                true
            }
            Err(_) => false,
        }
    }

    /// The session waits for an account (SRV-104), and its card says so.
    /// `stopped` when the pool stopped its process itself.
    fn wait(&self, nodus: &Nodus, session_id: &str, stopped: bool) {
        self.change_waiting(
            |waiting| match waiting.iter_mut().find(|w| w.session_id == session_id) {
                Some(entry) => entry.stopped |= stopped,
                None => waiting.push(Waiting {
                    session_id: session_id.to_owned(),
                    stopped,
                }),
            },
        );
        let note = match self.next_reset() {
            Some(at) => format!("Waiting for a Claude account until {}", reset_text(at, now_ms())),
            None => "Waiting for a Claude account".to_owned(),
        };
        nodus.note_account(session_id, Some(&note));
    }

    /// An account may be free: the waiting sessions resume, oldest first,
    /// while one is.
    async fn resume_waiting(&self, nodus: &Nodus) {
        // Each is tried once: one that may only go back to an account it
        // has run on (SRV-106) keeps its place, and the next is tried.
        let waiting = lock(&self.waiting).clone();
        for Waiting { session_id, stopped } in waiting {
            if !self.has_free() {
                return;
            }
            self.change_waiting(|waiting| waiting.retain(|w| w.session_id != session_id));
            // Someone stopped it meanwhile: it stays stopped.
            if !stopped && !nodus.is_live(&session_id) {
                nodus.note_account(&session_id, None);
                continue;
            }
            if !self.resume_on_another(nodus, &session_id, true).await {
                self.wait(nodus, &session_id, stopped);
            }
        }
    }

    /// When the next waiting session may take an account it hasn't run on
    /// (SRV-106).
    fn next_new_account(&self) -> Option<f64> {
        let now = now_ms();
        let seen = lock(&self.seen);
        lock(&self.waiting)
            .iter()
            .filter_map(|w| seen.get(&w.session_id)?.new_at)
            .map(|at| at + NEW_ACCOUNT_EVERY_MS)
            .filter(|&at| at > now)
            .reduce(f64::min)
    }

    /// SRV-104, SRV-105: a removed or failed account's live sessions move;
    /// with nowhere to go, a session mid-turn waits and the rest just stop,
    /// to start again on another account when they're next used. The
    /// session whose turn failed (`failed_turn`) counts as mid-turn, and is
    /// told to carry on.
    async fn move_off(&self, nodus: &Nodus, account_id: &str, failed_turn: Option<&str>) {
        for session in nodus.sessions_on_account(account_id) {
            let busy = session.busy || failed_turn == Some(session.session_id.as_str());
            if self.has_free() && self.resume_on_another(nodus, &session.session_id, busy).await {
                continue;
            }
            nodus.sessions().kill(&session.session_id).await;
            if busy {
                self.wait(nodus, &session.session_id, true);
            }
        }
    }
}

/// SRV-104, SRV-105: what the pool does with what it noted, in order, and
/// limited accounts freed when their windows reset. Runs while the server
/// does.
pub(crate) async fn run(pool: Arc<Pool>, nodus: Nodus) {
    let Some(mut todo) = lock(&pool.todo).take() else {
        return;
    };
    // A reset that passed while the server was down frees its account,
    // and sessions that waited then go on now, if one is free.
    pool.free_due();
    if !lock(&pool.waiting).is_empty() {
        let _ = pool.work.send(Work::Freed);
    }
    loop {
        let next = match (pool.next_reset(), pool.next_new_account()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let wait = next.map(|at| {
            let ms = (at - now_ms()).max(0.0);
            Duration::from_millis(ms as u64 + 50)
        });
        let work = tokio::select! {
            work = todo.recv() => match work {
                Some(work) => work,
                None => return,
            },
            () = sleep(wait) => {
                // A reset frees an account, which resumes who waits; else
                // a session's half hour (SRV-106) has passed.
                if !pool.free_due() {
                    pool.resume_waiting(&nodus).await;
                }
                continue;
            }
        };
        match work {
            Work::Changed => nodus.emit(Event::AccountsChanged {
                server_id: String::new(),
            }),
            Work::Limited { session_id } => pool.move_session(&nodus, &session_id).await,
            Work::Removed { account_id } => pool.move_off(&nodus, &account_id, None).await,
            Work::Failed { account_id, session_id } => {
                pool.move_off(&nodus, &account_id, Some(&session_id)).await;
            }
            Work::Freed => pool.resume_waiting(&nodus).await,
        }
    }
}

async fn sleep(wait: Option<Duration>) {
    match wait {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending().await,
    }
}

const NO_SUCH_ACCOUNT: &str = "No such Claude account";

fn check_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > LABEL_CHARS || label.chars().any(char::is_control) {
        return Err(format!("A label is 1 to {LABEL_CHARS} characters"));
    }
    Ok(label.to_owned())
}

/// A usage window from the status line: `used_percentage`, and
/// `resets_at` in seconds since the epoch, at most a week after `now`
/// (SRV-106).
fn window(value: Option<&Value>, now: f64) -> Option<UsageWindow> {
    let value = value?;
    let used = value.get("used_percentage").and_then(Value::as_f64)?;
    if !used.is_finite() {
        return None;
    }
    let resets_at = value
        .get("resets_at")
        .and_then(Value::as_f64)
        .filter(|at| at.is_finite() && *at > 0.0)
        .map(|at| (at * 1000.0).min(now + RESET_AHEAD_MS));
    Some(UsageWindow {
        used_percentage: used.clamp(0.0, 100.0),
        resets_at,
    })
}

/// SRV-101: why no account can take a process.
fn none_free(accounts: &[Stored], now: f64) -> String {
    let next = accounts
        .iter()
        .filter(|a| a.state == AccountState::Limited)
        .filter_map(|a| a.resets_at)
        .reduce(f64::min);
    match next {
        Some(at) => format!("Every Claude account is at its limit until {}.", reset_text(at, now)),
        None => "Every Claude account has stopped working. Add one in Settings → Servers.".to_owned(),
    }
}

/// A reset time in the server's words: `14:05 UTC`, with the day when
/// it's not within the next 20 hours (`Nov 14, 14:05 UTC`).
pub(crate) fn reset_text(at_ms: f64, now_ms: f64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = (at_ms / 1000.0).max(0.0) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let time = format!("{:02}:{:02} UTC", rest / 3600, rest % 3600 / 60);
    if at_ms - now_ms < 20.0 * 3600.0 * 1000.0 {
        return time;
    }
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{} {day}, {time}", MONTHS[(month - 1) as usize])
}

/// A file that can't be written doesn't stop what it records.
fn tracing_err(err: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "nodus-server: {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, five: Option<f64>, last: Option<f64>) -> Stored {
        Stored {
            id: id.into(),
            label: format!("Account {id}"),
            added_by: None,
            token: format!("tok-{id}"),
            added_at: 1.0,
            state: AccountState::Ok,
            resets_at: None,
            five_hour: five.map(|used| UsageWindow {
                used_percentage: used,
                resets_at: None,
            }),
            seven_day: None,
            last_used_at: last,
            failure: None,
        }
    }

    fn pool(accounts: &[Stored]) -> (tempfile::TempDir, Pool) {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(Auth::open(dir.path(), None, "box".into()).unwrap());
        store::write_json(&dir.path().join(ACCOUNTS_FILE), &accounts).unwrap();
        let pool = Pool::open(dir.path(), auth).unwrap();
        (dir, pool)
    }

    /// SRV-104: who waits outlives the pool.
    #[test]
    fn keeps_the_sessions_waiting_across_a_restart() {
        let (dir, pool) = pool(&[account("a", None, None)]);
        pool.change_waiting(|w| {
            w.push(Waiting {
                session_id: "s1".into(),
                stopped: false,
            });
            w.push(Waiting {
                session_id: "s2".into(),
                stopped: true,
            });
        });
        drop(pool);
        let auth = Arc::new(Auth::open(dir.path(), None, "box".into()).unwrap());
        let again = Pool::open(dir.path(), Arc::clone(&auth)).unwrap();
        let waiting = lock(&again.waiting).clone();
        assert_eq!(
            waiting
                .iter()
                .map(|w| (w.session_id.as_str(), w.stopped))
                .collect::<Vec<_>>(),
            [("s1", false), ("s2", true)]
        );
        again.change_waiting(|w| w.retain(|w| w.session_id != "s1"));
        drop(again);
        let kept = Pool::open(dir.path(), Arc::clone(&auth)).unwrap();
        assert_eq!(lock(&kept.waiting).len(), 1);
        // One that can't be read starts the server with nobody waiting.
        std::fs::write(dir.path().join(WAITING_FILE), "not json").unwrap();
        assert!(lock(&Pool::open(dir.path(), auth).unwrap().waiting).is_empty());
    }

    #[test]
    fn picks_the_least_used_ok_account_then_the_least_recently_used() {
        let mut limited = account("limited", Some(0.0), None);
        limited.state = AccountState::Limited;
        limited.resets_at = Some(now_ms() + 60_000.0);
        let mut failed = account("failed", None, None);
        failed.state = AccountState::Failed;
        let (_dir, pool) = pool(&[
            limited,
            failed,
            account("busy", Some(60.0), None),
            account("recent", Some(20.0), Some(now_ms())),
            // Unknown counts as 0, but it was used most recently.
            account("unknown", None, Some(now_ms() + 1.0)),
            account("old", Some(20.0), Some(5.0)),
        ]);
        // Unknown counts as 0, however recently it was used.
        assert_eq!(pool.pick(None).unwrap().id, "unknown");
        assert_eq!(pool.pick(None).unwrap().id, "unknown");
        // At its limit. Straight to `limit`, not through the hook: the hook
        // also starts moving the session, whose pick would race these.
        pool.limit("unknown");
        // A tie at 20% goes to the one used longer ago; each pick is a use.
        let picked = pool.pick(None).unwrap();
        assert_eq!(
            (picked.id.as_str(), picked.label.as_str(), picked.token.as_str()),
            ("old", "Account old", "tok-old")
        );
        assert_eq!(pool.pick(None).unwrap().id, "recent");
        assert_eq!(pool.pick(None).unwrap().id, "old");
    }

    #[test]
    fn a_session_gets_an_account_it_hasnt_run_on_once_in_half_an_hour() {
        let (_dir, pool) = pool(&[
            account("a", Some(10.0), None),
            account("b", Some(20.0), None),
            account("c", Some(30.0), None),
        ]);
        // Its first account, then one more, whenever.
        assert_eq!(pool.pick(Some("s1")).unwrap().id, "a");
        pool.limit("a");
        assert_eq!(pool.pick(Some("s1")).unwrap().id, "b");
        // SRV-106: a third within the half hour isn't given; back to one it
        // has run on, or nothing yet.
        pool.limit("b");
        assert_eq!(pool.pick(Some("s1")).unwrap_err(), NO_ACCOUNT_YET);
        lock(&pool.accounts)[0].state = AccountState::Ok;
        assert_eq!(pool.pick(Some("s1")).unwrap().id, "a");
        // Other sessions, and background runs, take what's free.
        pool.limit("a");
        assert_eq!(pool.pick(Some("s1")).unwrap_err(), NO_ACCOUNT_YET);
        assert_eq!(pool.pick(Some("s2")).unwrap().id, "c");
        assert_eq!(pool.pick(None).unwrap().id, "c");
        // Once the half hour has passed, it may.
        lock(&pool.seen).get_mut("s1").unwrap().new_at = Some(now_ms() - NEW_ACCOUNT_EVERY_MS - 1.0);
        assert_eq!(pool.pick(Some("s1")).unwrap().id, "c");
    }

    #[test]
    fn no_window_resets_more_than_a_week_ahead() {
        let now = now_ms();
        let far = json!({ "used_percentage": 100, "resets_at": 9_999_999_999.0_f64 });
        let window = window(Some(&far), now).unwrap();
        assert_eq!(window.resets_at, Some(now + RESET_AHEAD_MS));
        let mut a = account("a", None, None);
        a.seven_day = Some(window);
        assert!(a.limited_until(now) <= now + RESET_AHEAD_MS);
    }

    #[test]
    fn says_why_nothing_can_be_picked() {
        let (_dir, empty) = pool(&[]);
        assert_eq!(empty.pick(None).unwrap_err(), NO_ACCOUNTS);
        let now = now_ms();
        let mut a = account("a", None, None);
        a.state = AccountState::Limited;
        a.resets_at = Some(now + 7_200_000.0);
        let mut b = a.clone();
        b.id = "b".into();
        b.resets_at = Some(now + 3_600_000.0);
        let (_dir, limited) = pool(&[a, b.clone()]);
        let err = limited.pick(None).unwrap_err();
        assert_eq!(
            err,
            format!(
                "Every Claude account is at its limit until {}.",
                reset_text(now + 3_600_000.0, now)
            )
        );
        b.state = AccountState::Failed;
        let (_dir, failed) = pool(&[b]);
        assert!(
            failed
                .pick(None)
                .unwrap_err()
                .starts_with("Every Claude account has stopped working")
        );
    }

    #[test]
    fn a_limited_account_frees_itself_when_its_window_resets() {
        let mut a = account("a", None, None);
        a.state = AccountState::Limited;
        a.resets_at = Some(now_ms() - 1.0);
        let (_dir, pool) = pool(&[a]);
        assert_eq!(pool.pick(None).unwrap().id, "a");
        assert_eq!(pool.list_raw()[0].state, AccountState::Ok);
    }

    #[test]
    fn reads_usage_and_limits_from_what_sessions_report() {
        let (dir, pool) = pool(&[account("a", None, None)]);
        let now = now_ms();
        let resets = (now / 1000.0).floor() + 3600.0;
        pool.session_hook(
            "s1",
            Some("a"),
            "StatusLine",
            &json!({ "rate_limits": {
                "five_hour": { "used_percentage": 42.5, "resets_at": resets },
                "seven_day": { "used_percentage": 12 }
            }}),
        );
        let a = &pool.list_raw()[0];
        assert_eq!(
            a.five_hour,
            Some(UsageWindow {
                used_percentage: 42.5,
                resets_at: Some(resets * 1000.0)
            })
        );
        assert_eq!(a.seven_day.map(|w| w.used_percentage), Some(12.0));
        // Not this account, and not a report: nothing.
        pool.session_hook("s1", None, "StopFailure", &json!({ "error": "rate_limit" }));
        pool.session_hook("s1", Some("a"), "StopFailure", &json!({ "error": "server_error" }));
        assert_eq!(pool.list_raw()[0].state, AccountState::Ok);

        pool.session_hook("s1", Some("a"), "StopFailure", &json!({ "error": "rate_limit" }));
        let a = &pool.list_raw()[0];
        assert_eq!(a.state, AccountState::Limited);
        assert_eq!(a.resets_at, Some(resets * 1000.0));

        pool.session_hook(
            "s1",
            Some("a"),
            "StopFailure",
            &json!({ "error": "authentication_failed" }),
        );
        let a = &pool.list_raw()[0];
        assert_eq!(a.state, AccountState::Failed);
        assert_eq!(a.failure.as_deref(), Some("authentication_failed"));
        // Kept, 0600, with its token: the file is the server's alone.
        let kept = std::fs::read_to_string(dir.path().join(ACCOUNTS_FILE)).unwrap();
        assert!(
            kept.contains("\"token\": \"tok-a\"") && kept.contains("\"state\": \"failed\""),
            "{kept}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(ACCOUNTS_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_removed_members_accounts_can_go_with_them() {
        let by = |github_id: u64, login: &str| {
            Some(AuditMember {
                github_id,
                login: login.into(),
            })
        };
        let mut alices = account("a", None, None);
        alices.added_by = by(1, "alice");
        let mut alices_too = account("b", None, None);
        alices_too.added_by = by(1, "alice");
        let mut bobs = account("c", None, None);
        bobs.added_by = by(2, "bob");
        let (dir, pool) = pool(&[alices, bobs, alices_too]);
        pool.remove_added_by(&Who::Dev, 1);
        let left: Vec<String> = pool.list_raw().into_iter().map(|a| a.id).collect();
        assert_eq!(left, ["c"]);
        let audit = std::fs::read_to_string(dir.path().join(crate::audit::AUDIT_FILE)).unwrap();
        assert_eq!(audit.matches("account.removed").count(), 2);
        assert!(!audit.contains("tok-"), "{audit}");
    }

    #[test]
    fn a_limit_lasts_until_the_window_resets_or_an_hour() {
        let now = 1_000_000.0;
        let mut a = account("a", Some(100.0), None);
        assert_eq!(a.limited_until(now), now + LIMIT_UNKNOWN_MS);
        a.five_hour = Some(UsageWindow {
            used_percentage: 100.0,
            resets_at: Some(now + 5.0),
        });
        a.seven_day = Some(UsageWindow {
            used_percentage: 80.0,
            resets_at: Some(now + 50.0),
        });
        assert_eq!(a.limited_until(now), now + 5.0);
        a.seven_day = Some(UsageWindow {
            used_percentage: 100.0,
            resets_at: Some(now + 50.0),
        });
        assert_eq!(a.limited_until(now), now + 50.0);
    }

    #[test]
    fn says_a_reset_time_in_utc() {
        // 2026-11-13 14:05:00 UTC.
        let at = 1_794_578_700_000.0;
        assert_eq!(reset_text(at, at - 60_000.0), "14:05 UTC");
        assert_eq!(reset_text(at, at - 3.0 * 86_400_000.0), "Nov 13, 14:05 UTC");
    }

    impl Pool {
        fn list_raw(&self) -> Vec<Stored> {
            lock(&self.accounts).clone()
        }
    }
}
