//! Session status driven by Claude Code hooks.
//!
//! Every claude process the app starts gets `--settings` registering hooks
//! that POST the hook payload to the session host's HTTP server
//! ([`HookEndpoint::settings`]). The `NODUS_*` variables in the process's
//! environment ([`HookEndpoint::env`]) tell the hook which card it belongs to.
//! The host passes each hook on to the app, which feeds it to the
//! [`StatusMachine`].
//!
//! Hooks fired inside a subagent carry its `agent_id`. Those drive the agent's
//! own status, and a card reads as working (or waiting) while its main thread
//! or any of its agents is.
//!
//! The same settings take over Claude's status line: it posts what it would
//! show (model, context, cost) and prints nothing, so the card shows it
//! instead and the terminal keeps the row ([`crate::usage`]).
//!
//! Ports `src/main/hooks.ts`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use nodus_domain::{Event, EventSink, SessionStatus, SessionUsage};
use nodus_platform::Env;
use nodus_platform::env::EnvMap;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use crate::jsvalue::{get, str_of, string};
use crate::usage::{to_usage, usage_key};

/// The hooks that drive status, in the order the settings register them (HOST-138).
pub const HOOK_EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SessionEnd",
    "SubagentStart",
    "SubagentStop",
];

/// The event the host passes a status line report on as (hostProtocol.ts's `STATUS_LINE_EVENT`).
pub const STATUS_LINE_EVENT: &str = "StatusLine";

/// How long after Esc or Ctrl+C a turn with no hook since counts as interrupted (HOST-163).
pub const INTERRUPT_MS: u64 = 1200;

/// The time hooks and keystrokes are stamped with, in milliseconds since the
/// Unix epoch, as `Date.now()` gives it.
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
}

/// The wall clock, as the TypeScript's `Date.now()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        // Date.now() is whole milliseconds.
        since.as_millis() as f64
    }
}

/// A clock that starts at a given time and moves with tokio's, so a test on
/// a paused runtime moves both with `tokio::time::advance` or a sleep, as
/// `vi.useFakeTimers` moves `Date.now()` with the timers.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock {
    start: f64,
    since: tokio::time::Instant,
}

impl TokioClock {
    pub fn starting_at(start: f64) -> Self {
        Self {
            start,
            since: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> f64 {
        self.start + self.since.elapsed().as_nanos() as f64 / 1e6
    }
}

/// What hooks have said about an agent in its session's current process (HOST-153).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgentHook {
    pub status: SessionStatus,
    /// When the agent's last hook arrived.
    pub at: f64,
    /// When it took on this status.
    pub since: f64,
}

/// Whether an agent, by its transcript, has background work in flight that
/// will wake it (agents.rs `has_pending_work`).
pub type PendingWork = dyn Fn(&str) -> bool + Send + Sync;

/// The status of every session and its agents, and their usage.
///
/// It's fed by the session host client: [`process_started`] and
/// [`process_exited`] as it starts and stops tracking a session,
/// [`receive_hook`] for each hook the host passes on (and each one replayed
/// from a welcome), and [`note_user_input`] for each write to a session. It
/// reports changes as [`Event::Status`], [`Event::AgentStatus`] and
/// [`Event::Usage`], after it has let go of its own state, so whoever handles
/// them can ask it anything.
///
/// A session is live from [`process_started`] to [`process_exited`], which the
/// client calls exactly when it starts and stops tracking the session, so
/// [`is_live`] says what pty.ts's `isLive` says.
///
/// Cheap to clone; clones share their state.
///
/// [`process_started`]: StatusMachine::process_started
/// [`process_exited`]: StatusMachine::process_exited
/// [`receive_hook`]: StatusMachine::receive_hook
/// [`note_user_input`]: StatusMachine::note_user_input
/// [`is_live`]: StatusMachine::is_live
#[derive(Clone)]
pub struct StatusMachine {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    sink: Arc<dyn EventSink>,
    clock: Arc<dyn Clock>,
    pending_work: Arc<PendingWork>,
    /// Where interrupt timers run when the caller isn't on a runtime.
    runtime: Option<Handle>,
}

#[derive(Default)]
struct State {
    /// Status of each session's main thread.
    statuses: HashMap<String, SessionStatus>,
    last_emitted: HashMap<String, (SessionStatus, bool)>,
    interrupts: HashMap<String, Interrupt>,
    next_interrupt: u64,
    /// Last hook from a session's main thread; agents' hooks don't count.
    last_hook_at: HashMap<String, f64>,
    /// Agents seen through hooks in each session's current process, by agent
    /// id, in the order they were first seen.
    agents: HashMap<String, IndexMap<String, AgentHook>>,
    /// Each session's latest status line report, kept after its process exits.
    usages: HashMap<String, SessionUsage>,
    last_usage_key: HashMap<String, String>,
    /// Sessions whose process is running (pty.ts's `running`).
    live: HashSet<String>,
}

struct Interrupt {
    id: u64,
    task: Option<AbortHandle>,
}

impl Interrupt {
    fn cancel(self) {
        if let Some(task) = self.task {
            task.abort();
        }
    }
}

/// How a step ended: the events it gave, to send once the state is let go.
type Events = Vec<Event>;

impl StatusMachine {
    /// A status machine reporting to `sink`, stamping live hooks and
    /// keystrokes with `clock`, and asking `pending_work` whether an agent
    /// that stopped will be woken again. Interrupt timers run on the tokio
    /// runtime it's made on, or the caller's.
    pub fn new(
        sink: Arc<dyn EventSink>,
        clock: Arc<dyn Clock>,
        pending_work: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                sink,
                clock,
                pending_work: Arc::new(pending_work),
                runtime: Handle::try_current().ok(),
            }),
        }
    }

    /// Whether the session's process is running.
    pub fn is_live(&self, session_id: &str) -> bool {
        self.lock().live.contains(session_id)
    }

    /// The session's status: its main thread's, raised by any agent working
    /// or waiting, while its process runs; `done` otherwise (HOST-133).
    pub fn status(&self, session_id: &str) -> SessionStatus {
        self.lock().status(session_id)
    }

    /// What hooks have said about an agent in the session's current process, if anything.
    pub fn agent_hook(&self, session_id: &str, agent_id: &str) -> Option<AgentHook> {
        self.lock().agents.get(session_id)?.get(agent_id).copied()
    }

    /// The session's latest usage, kept after its process exits (HOST-173).
    pub fn usage(&self, session_id: &str) -> Option<SessionUsage> {
        self.lock().usages.get(session_id).cloned()
    }

    /// Sets the main thread's status and reports the session's if it changed.
    /// `live` overrides whether the process is running for the report.
    pub fn set_status(&self, session_id: &str, status: SessionStatus, live: Option<bool>) {
        self.step(|state, events| state.set_status(session_id, status, live, events));
    }

    /// HOST-135: the session's process started, or was adopted from the host,
    /// in status `initial` (`done` for a process with nothing to do yet).
    pub fn process_started(&self, session_id: &str, initial: SessionStatus) {
        let now = self.now();
        self.step(|state, events| {
            state.live.insert(session_id.to_owned());
            state.mark_process(session_id, true, initial, now, events);
        });
    }

    /// HOST-136: the session's process exited.
    pub fn process_exited(&self, session_id: &str) {
        let now = self.now();
        self.step(|state, events| {
            state.live.remove(session_id);
            state.mark_process(session_id, false, SessionStatus::Done, now, events);
        });
    }

    /// HOST-137: forgets everything about a session (card removed, run
    /// finished, brain stopped). A pending interrupt timer is left to fire,
    /// harmlessly.
    pub fn forget(&self, session_id: &str) {
        let mut state = self.lock();
        state.statuses.remove(session_id);
        state.last_emitted.remove(session_id);
        state.last_hook_at.remove(session_id);
        state.agents.remove(session_id);
        state.usages.remove(session_id);
        state.last_usage_key.remove(session_id);
    }

    /// A hook fired by a session's process, passed on by the session host
    /// (HOST-138). `at` is when it fired: hooks replayed after a relaunch keep
    /// the time the host recorded, live ones take the clock's (HOST-158).
    pub fn receive_hook(&self, event: &str, session_id: &str, payload: &Value, at: Option<f64>) {
        let at = at.unwrap_or_else(|| self.now());
        // `payload ?? {}`
        let empty = Value::Object(Default::default());
        let payload = if payload.is_null() { &empty } else { payload };

        if event == STATUS_LINE_EVENT {
            let usage = to_usage(payload, at);
            return self.step(|state, events| state.set_usage(session_id, usage, events));
        }
        if !HOOK_EVENTS.contains(&event) {
            return;
        }
        let agent_id = str_of(get(payload, "agent_id")).filter(|id| !id.is_empty());
        // Asked before taking the state: it reads the agent's transcript.
        let waking = match (agent_id, event) {
            (Some(_), "SubagentStop") => {
                str_of(get(payload, "agent_transcript_path")).is_some_and(|file| (self.shared.pending_work)(file))
            }
            _ => false,
        };
        self.step(|state, events| match agent_id {
            Some(agent_id) => state.agent_hook(event, session_id, agent_id, payload, waking, at, events),
            None => state.main_hook(event, session_id, payload, at, events),
        });
    }

    /// Keystroke heuristics for transitions that fire no hook: Stop doesn't
    /// run when the user interrupts (Esc, Ctrl+C), and approving a permission
    /// prompt fires nothing until the tool finishes. `at` is when the key was
    /// pressed: keys replayed after a relaunch keep the time the host
    /// recorded, live ones take the clock's.
    pub fn note_user_input(&self, session_id: &str, data: &str, at: Option<f64>) {
        let at = at.unwrap_or_else(|| self.now());
        let mut state = self.lock();
        let current = state.statuses.get(session_id).copied();
        // HOST-160
        let main_active = matches!(current, Some(SessionStatus::Working | SessionStatus::Waiting));
        let waiting_agent = state.longest_waiting_agent(session_id);
        if !main_active && waiting_agent.is_none() {
            return;
        }

        // HOST-161: one keystroke per write, as xterm sends them.
        let answer = data == "\r" || is_digit_key(data);
        let interrupt = data == "\x1b" || data == "\x03";

        // HOST-162: answering an agent's prompt, either way, lets it carry on.
        // Prompts queue up, so the answer is for the one that has waited longest.
        if current != Some(SessionStatus::Waiting)
            && let Some(agent_id) = waiting_agent
            && (answer || interrupt)
        {
            let mut events = Events::new();
            state.set_agent_status(session_id, &agent_id, SessionStatus::Working, at, &mut events);
            drop(state);
            return self.send(events);
        }

        // HOST-163, HOST-165
        if interrupt {
            state.next_interrupt += 1;
            let id = state.next_interrupt;
            let task = self.start_interrupt_timer(session_id, id, at);
            if let Some(previous) = state.interrupts.insert(session_id.to_owned(), Interrupt { id, task }) {
                previous.cancel();
            }
            return;
        }

        // HOST-164
        if current == Some(SessionStatus::Waiting) && answer {
            let mut events = Events::new();
            state.set_status(session_id, SessionStatus::Working, None, &mut events);
            drop(state);
            self.send(events);
        }
    }

    /// HOST-156: marks an agent done that ended without its SubagentStop
    /// reaching the app (one stopped from Claude's task list, say). Its
    /// transcript is what tells.
    pub fn settle_agent(&self, session_id: &str, agent_id: &str) {
        let now = self.now();
        self.step(|state, events| {
            let known = state.agents.get(session_id).and_then(|agents| agents.get(agent_id));
            if known.is_some_and(|agent| agent.status != SessionStatus::Done) {
                state.set_agent_status(session_id, agent_id, SessionStatus::Done, now, events);
            }
        });
    }

    /// HOST-157: keeps an agent working that ended its turn with background
    /// work in flight: its SubagentStop has fired, but that work will wake it.
    pub fn hold_agent(&self, session_id: &str, agent_id: &str) {
        let now = self.now();
        self.step(|state, events| {
            let known = state.agents.get(session_id).and_then(|agents| agents.get(agent_id));
            if known.is_some_and(|agent| agent.status == SessionStatus::Done) {
                state.set_agent_status(session_id, agent_id, SessionStatus::Working, now, events);
            }
        });
    }

    fn start_interrupt_timer(&self, session_id: &str, id: u64, at: f64) -> Option<AbortHandle> {
        let Some(runtime) = self.shared.runtime.clone().or_else(|| Handle::try_current().ok()) else {
            tracing::warn!("no tokio runtime for the interrupt timer of {session_id}");
            return None;
        };
        let shared: Weak<Shared> = Arc::downgrade(&self.shared);
        let session_id = session_id.to_owned();
        let task = runtime.spawn(async move {
            tokio::time::sleep(Duration::from_millis(INTERRUPT_MS)).await;
            if let Some(shared) = shared.upgrade() {
                StatusMachine { shared }.interrupt_fired(&session_id, id, at);
            }
        });
        Some(task.abort_handle())
    }

    fn interrupt_fired(&self, session_id: &str, id: u64, at: f64) {
        self.step(|state, events| {
            // A newer keystroke or a new process replaced this timer.
            if state.interrupts.get(session_id).is_none_or(|current| current.id != id) {
                return;
            }
            state.interrupts.remove(session_id);
            // Any hook after the keypress means Claude kept going.
            if state.last_hook_at.get(session_id).copied().unwrap_or(0.0) > at {
                return;
            }
            if matches!(
                state.statuses.get(session_id),
                Some(SessionStatus::Working | SessionStatus::Waiting)
            ) {
                state.set_status(session_id, SessionStatus::Done, None, events);
            }
        });
    }

    fn now(&self) -> f64 {
        self.shared.clock.now()
    }

    /// Changes the state, then sends what changed once the state is let go.
    fn step(&self, change: impl FnOnce(&mut State, &mut Events)) {
        let mut events = Events::new();
        change(&mut self.lock(), &mut events);
        self.send(events);
    }

    fn send(&self, events: Events) {
        for event in events {
            self.shared.sink.emit(event);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the session host client tells the status machine, as pty.ts calls
/// hooks.ts. [`StatusMachine`] is one; a test of the client can pass a fake.
pub trait SessionInput: Send + Sync {
    /// The client started tracking a session: spawned, or adopted from a welcome.
    fn process_started(&self, session_id: &str, initial: SessionStatus);
    /// The client stopped tracking a session: it exited, or the host was lost.
    fn process_exited(&self, session_id: &str);
    /// A `hook` message, or a hook or status line replayed from a welcome with its recorded time.
    fn hook(&self, event: &str, session_id: &str, payload: &Value, at: Option<f64>);
    /// A write to a session, before it's sent, or an `input` replayed from a welcome with its recorded time.
    fn user_input(&self, session_id: &str, data: &str, at: Option<f64>);
}

impl SessionInput for StatusMachine {
    fn process_started(&self, session_id: &str, initial: SessionStatus) {
        StatusMachine::process_started(self, session_id, initial);
    }

    fn process_exited(&self, session_id: &str) {
        StatusMachine::process_exited(self, session_id);
    }

    fn hook(&self, event: &str, session_id: &str, payload: &Value, at: Option<f64>) {
        self.receive_hook(event, session_id, payload, at);
    }

    fn user_input(&self, session_id: &str, data: &str, at: Option<f64>) {
        self.note_user_input(session_id, data, at);
    }
}

/// `/^[1-9]$/`: the menu choices in Claude's prompts.
fn is_digit_key(data: &str) -> bool {
    matches!(data.as_bytes(), [b'1'..=b'9'])
}

/// HOST-145: a notification about permission, by its type or its message.
fn is_permission_notification(payload: &Value) -> bool {
    let kind = string_or_empty(get(payload, "notification_type"));
    let message = string_or_empty(get(payload, "message"));
    // `/permission/i` without the u flag folds ASCII letters only.
    kind == "permission_prompt" || kind == "elicitation_dialog" || message.to_ascii_lowercase().contains("permission")
}

/// `String(value ?? '')`.
fn string_or_empty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        value => string(value),
    }
}

impl State {
    /// HOST-133: the main thread's status, raised by any agent that is working or waiting.
    fn combined(&self, session_id: &str) -> SessionStatus {
        let main = self.statuses.get(session_id).copied().unwrap_or(SessionStatus::Done);
        let agents = self
            .agents
            .get(session_id)
            .into_iter()
            .flat_map(|agents| agents.values());
        let all: Vec<SessionStatus> = std::iter::once(main).chain(agents.map(|agent| agent.status)).collect();
        if all.contains(&SessionStatus::Waiting) {
            SessionStatus::Waiting
        } else if all.contains(&SessionStatus::Working) {
            SessionStatus::Working
        } else {
            SessionStatus::Done
        }
    }

    fn status(&self, session_id: &str) -> SessionStatus {
        if self.live.contains(session_id) {
            self.combined(session_id)
        } else {
            SessionStatus::Done
        }
    }

    /// HOST-134, HOST-159: reports the session's status when it changed.
    fn emit(&mut self, session_id: &str, live: Option<bool>, events: &mut Events) {
        let live = live.unwrap_or_else(|| self.live.contains(session_id));
        let status = if live {
            self.combined(session_id)
        } else {
            SessionStatus::Done
        };
        if self.last_emitted.get(session_id) == Some(&(status, live)) {
            return;
        }
        self.last_emitted.insert(session_id.to_owned(), (status, live));
        events.push(Event::Status {
            session_id: session_id.to_owned(),
            status,
            live,
        });
    }

    fn set_status(&mut self, session_id: &str, status: SessionStatus, live: Option<bool>, events: &mut Events) {
        self.statuses.insert(session_id.to_owned(), status);
        self.emit(session_id, live, events);
    }

    /// HOST-153, HOST-154
    fn set_agent_status(
        &mut self,
        session_id: &str,
        agent_id: &str,
        status: SessionStatus,
        at: f64,
        events: &mut Events,
    ) {
        let agents = self.agents.entry(session_id.to_owned()).or_default();
        let previous = agents.get(agent_id).copied();
        let since = match previous {
            Some(previous) if previous.status == status => previous.since,
            _ => at,
        };
        agents.insert(agent_id.to_owned(), AgentHook { status, at, since });
        if previous.is_none_or(|previous| previous.status != status) {
            events.push(Event::AgentStatus {
                session_id: session_id.to_owned(),
                agent_id: agent_id.to_owned(),
                status,
            });
        }
        self.emit(session_id, None, events);
    }

    /// HOST-155: the agent that has been waiting on the user the longest, if
    /// any is; on a tie, the one seen first.
    fn longest_waiting_agent(&self, session_id: &str) -> Option<String> {
        let mut found: Option<(&String, f64)> = None;
        for (id, agent) in self.agents.get(session_id)? {
            if agent.status == SessionStatus::Waiting && found.is_none_or(|(_, since)| agent.since < since) {
                found = Some((id, agent.since));
            }
        }
        found.map(|(id, _)| id.clone())
    }

    /// HOST-135, HOST-136
    fn mark_process(&mut self, session_id: &str, live: bool, initial: SessionStatus, now: f64, events: &mut Events) {
        if let Some(interrupt) = self.interrupts.remove(session_id) {
            interrupt.cancel();
        }
        let status = if live { initial } else { SessionStatus::Done };
        self.statuses.insert(session_id.to_owned(), status);
        // Agents only run inside the process; a new one starts with none.
        self.agents.remove(session_id);
        self.emit(session_id, Some(live), events);
        self.pause_usage(session_id, live, now, events);
    }

    /// HOST-139, HOST-150 to HOST-152: a hook from inside an agent drives that agent only.
    #[allow(clippy::too_many_arguments)]
    fn agent_hook(
        &mut self,
        event: &str,
        session_id: &str,
        agent_id: &str,
        payload: &Value,
        waking: bool,
        at: f64,
        events: &mut Events,
    ) {
        let status = match event {
            // An agent that ended its turn with a command still running in the background will be woken by it.
            "SubagentStop" if waking => SessionStatus::Working,
            "SubagentStop" => SessionStatus::Done,
            "PermissionRequest" => SessionStatus::Waiting,
            "PreToolUse" if is_ask_user_question(payload) => SessionStatus::Waiting,
            "Notification" if is_permission_notification(payload) => SessionStatus::Waiting,
            "SubagentStart" | "PostToolUse" | "UserPromptSubmit" | "PreToolUse" => SessionStatus::Working,
            // Its other notifications, and its Stop, SessionStart and SessionEnd.
            _ => return,
        };
        self.set_agent_status(session_id, agent_id, status, at, events);
    }

    /// HOST-140 to HOST-149: a hook from the session's main thread.
    fn main_hook(&mut self, event: &str, session_id: &str, payload: &Value, at: f64, events: &mut Events) {
        self.last_hook_at.insert(session_id.to_owned(), at);
        let current = self.statuses.get(session_id).copied().unwrap_or(SessionStatus::Done);
        match event {
            "UserPromptSubmit" | "PostToolUse" => self.set_status(session_id, SessionStatus::Working, None, events),
            "PreToolUse" => {
                let status = if is_ask_user_question(payload) {
                    SessionStatus::Waiting
                } else {
                    SessionStatus::Working
                };
                self.set_status(session_id, status, None, events);
            }
            "PermissionRequest" => self.set_status(session_id, SessionStatus::Waiting, None, events),
            "Notification" => {
                // An agent's permission prompt shows in the main terminal too, and that agent already reads as waiting.
                if is_permission_notification(payload) && self.longest_waiting_agent(session_id).is_none() {
                    self.set_status(session_id, SessionStatus::Waiting, None, events);
                }
            }
            "Stop" => self.set_status(session_id, SessionStatus::Done, None, events),
            "SessionStart" => {
                if current != SessionStatus::Working {
                    self.set_status(session_id, SessionStatus::Done, None, events);
                }
            }
            "SessionEnd" => {
                // /clear ends one session and starts another in the same process.
                let live = (get(payload, "reason").and_then(Value::as_str) != Some("clear")).then_some(false);
                self.set_status(session_id, SessionStatus::Done, live, events);
            }
            // SubagentStart and SubagentStop without an agent only count as a hook.
            _ => {}
        }
    }

    /// HOST-171: the renderer counts the running time on from the last
    /// report, so a report that only moved the clock is kept without being sent.
    fn set_usage(&mut self, session_id: &str, usage: SessionUsage, events: &mut Events) {
        let key = usage_key(&usage);
        self.usages.insert(session_id.to_owned(), usage.clone());
        if self.last_usage_key.get(session_id) == Some(&key) {
            return;
        }
        self.last_usage_key.insert(session_id.to_owned(), key);
        events.push(Event::Usage {
            session_id: session_id.to_owned(),
            usage,
        });
    }

    /// HOST-172: the running time stops with the process and picks up when it
    /// starts again, as Claude counts it.
    fn pause_usage(&mut self, session_id: &str, live: bool, at: f64, events: &mut Events) {
        let Some(usage) = self.usages.get_mut(session_id) else {
            return;
        };
        // The new process's first report has its own count; send it whatever it says.
        if live {
            self.last_usage_key.remove(session_id);
        }
        if !live && let Some(duration) = usage.duration_ms {
            usage.duration_ms = Some(duration + at - usage.at);
        }
        usage.at = at;
        events.push(Event::Usage {
            session_id: session_id.to_owned(),
            usage: usage.clone(),
        });
    }
}

fn is_ask_user_question(payload: &Value) -> bool {
    get(payload, "tool_name").and_then(Value::as_str) == Some("AskUserQuestion")
}

// ---------------------------------------------------------------------------
// The endpoint
//
// The hook server runs in the session host, so it outlives the app along with
// the processes posting to it.

/// Where hooks reach the session host, and the settings and environment that
/// point Claude's hooks there.
///
/// Before a host is adopted it's port 0 and an empty token (HOST-174).
#[derive(Debug, Default)]
pub struct HookEndpoint {
    endpoint: Mutex<Endpoint>,
    /// HOST-181: the settings JSON by how hooks reach the host, built once each.
    settings: Mutex<HashMap<(bool, bool), Arc<str>>>,
}

#[derive(Debug, Default, Clone)]
struct Endpoint {
    port: u16,
    token: String,
    /// The curl config holding the token, when the host takes it in a header.
    config: Option<String>,
}

impl HookEndpoint {
    pub fn new() -> Self {
        Self::default()
    }

    /// HOST-106: the adopted host's hook port and token, and the curl config
    /// its token was written to, if it could be.
    pub fn set(&self, port: u16, token: &str, config: Option<&str>) {
        *lock(&self.endpoint) = Endpoint {
            port,
            token: token.to_owned(),
            // An empty path counts as none, as the TypeScript tests it for truth.
            config: config.filter(|c| !c.is_empty()).map(str::to_owned),
        };
    }

    /// HOST-176: what a Claude process needs in its environment so its hooks
    /// can reach the host.
    pub fn env(&self, session_id: &str) -> EnvMap {
        let endpoint = lock(&self.endpoint).clone();
        let mut env = EnvMap::new();
        env.insert("NODUS_PORT".into(), endpoint.port.to_string());
        env.insert("NODUS_TOKEN".into(), endpoint.token);
        env.insert("NODUS_SESSION".into(), session_id.to_owned());
        if let Some(config) = endpoint.config {
            env.insert("NODUS_HOOK_CONFIG".into(), config);
        }
        env
    }

    /// HOST-175, HOST-180: the `--settings` JSON registering the hooks and
    /// the status line, identical for every process on a host. Flag settings
    /// outrank the user's, so their own status line makes way for Nodus's.
    /// `powershell` is [`uses_powershell`].
    pub fn settings(&self, powershell: bool) -> Arc<str> {
        let via_config = lock(&self.endpoint).config.is_some();
        let mut cache = lock(&self.settings);
        cache
            .entry((via_config, powershell))
            .or_insert_with(|| hook_settings(via_config, powershell).into())
            .clone()
    }
}

/// PowerShell on Windows without Git Bash; a POSIX shell everywhere else
/// (the claude command points claude at that bash).
pub fn uses_powershell(env: &Env) -> bool {
    cfg!(windows) && env.git_bash().is_none()
}

/// HOST-177, HOST-178, HOST-182: a command posting its stdin to the host
/// under `event`, and printing nothing. curl reads the token from the config
/// file, as anything on its command line shows up in process listings. An
/// older host only takes it in the URL.
fn post_command(event: &str, via_config: bool) -> String {
    let target = if via_config {
        format!(r#"-K "$NODUS_HOOK_CONFIG" "http://127.0.0.1:$NODUS_PORT/hook?evt={event}&sid=$NODUS_SESSION" "#)
    } else {
        format!(r#""http://127.0.0.1:$NODUS_PORT/hook?evt={event}&sid=$NODUS_SESSION&token=$NODUS_TOKEN" "#)
    };
    format!("curl -s -m 2 -X POST -H 'Content-Type: application/json' --data-binary @- {target}>/dev/null 2>&1; exit 0")
}

/// HOST-179: the same in PowerShell, which Claude Code runs hooks in on
/// Windows when Git Bash isn't there. The token comes from the environment,
/// which no process listing shows, and stdin is sent as it came, so nothing
/// re-encodes it. Only hosts that take the token in a header run on Windows.
fn powershell_post_command(event: &str) -> String {
    format!(
        "$ProgressPreference = 'SilentlyContinue'; $body = [IO.MemoryStream]::new(); \
         [Console]::OpenStandardInput().CopyTo($body); \
         try {{ $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -ContentType 'application/json' \
         -Headers @{{ Authorization = \"Bearer $env:NODUS_TOKEN\" }} -Body $body.ToArray() \
         -Uri \"http://127.0.0.1:$env:NODUS_PORT/hook?evt={event}&sid=$env:NODUS_SESSION\" }} catch {{}}; exit 0"
    )
}

/// HOST-180: `JSON.stringify({hooks, statusLine})`.
fn hook_settings(via_config: bool, powershell: bool) -> String {
    let command = |event: &str, last: (&str, Value)| {
        let mut entry = serde_json::Map::new();
        entry.insert("type".into(), json!("command"));
        if powershell {
            entry.insert("command".into(), json!(powershell_post_command(event)));
            entry.insert("shell".into(), json!("powershell"));
        } else {
            entry.insert("command".into(), json!(post_command(event, via_config)));
        }
        entry.insert(last.0.into(), last.1);
        Value::Object(entry)
    };
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        // Output must stay empty: UserPromptSubmit stdout is injected into Claude's context.
        hooks.insert(
            event.into(),
            json!([{ "hooks": [command(event, ("timeout", json!(5)))] }]),
        );
    }
    // With no output, the terminal shows no status line.
    let status_line = command(STATUS_LINE_EVENT, ("padding", json!(0)));
    nodus_domain::json::to_string(&json!({ "hooks": hooks, "statusLine": status_line }))
        .expect("the settings are plain JSON")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
