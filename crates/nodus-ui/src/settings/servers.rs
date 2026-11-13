//! Settings → Servers, one server's people, Claude accounts and GitHub
//! (SRV-165, SRV-029, SRV-063, SRV-105, SRV-210; UI-323…UI-327,
//! UI-330…UI-332): its members, its devices, its pool of Claude accounts,
//! its GitHub App and installations, and its recent activity, each as the
//! server answers it now. Removing and revoking ask first, in place.

mod accounts;

pub use accounts::TOKEN_HELP;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Global, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_core::hosted::{AuditLine, ClaudeAccount, Device, Installation, Member, ServerDetails, install_url};
use nodus_domain::{ServerInfo, ServerState};

use super::{group, plain_row, row_text};
use crate::components::button::{Kind, button};
use crate::components::icon::{Icon, icon};
use crate::components::text_field::new_input;
use crate::servers::{avatar, open_url};
use crate::state::AppState;
use crate::theme::tokens::*;

/// How many lines of activity show before **Show all**.
const ACTIVITY_SHOWN: usize = 8;

/// What a `.pem` picker answers instead of showing, set by the test driver
/// (`{"cmd":"stub","name":"pickFile"}`).
pub struct PickFileStub(pub Option<PathBuf>);

impl Global for PickFileStub {}

/// Something asked of the server: on its way, there, or why not.
enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

/// A removal waiting for its confirmation.
#[derive(Clone, PartialEq)]
enum Confirm {
    Member(u64),
    Device(String),
    Account(String),
}

pub struct ServerManager {
    state: Entity<AppState>,
    server_id: String,
    /// Whether it was connected when last asked, to ask again on coming back.
    asked_connected: bool,
    members: Load<Vec<Member>>,
    devices: Load<Vec<Device>>,
    audit: Load<Vec<AuditLine>>,
    info: Load<ServerDetails>,
    invite: Entity<InputState>,
    inviting: bool,
    /// Why the last invite, removal or revoke failed.
    error: Option<String>,
    confirm: Option<Confirm>,
    /// Replacing the App key: under way, and how it went.
    replacing: bool,
    key_note: Option<(bool, String)>,
    all_activity: bool,
    avatars: HashMap<String, Option<PathBuf>>,
    /// Empties the invite field on the next draw.
    clear_invite: bool,
    /// The pool of Claude accounts (SRV-105).
    accounts: Load<Vec<ClaudeAccount>>,
    /// The `accounts-changed` and notice counts last asked after.
    accounts_seen: u64,
    notices_seen: u64,
    account_label: Entity<InputState>,
    account_token: Entity<InputState>,
    /// The Add form is open, and a token is being tried.
    adding_account: bool,
    checking: bool,
    /// Why the last add, rename or removal of an account failed.
    account_error: Option<String>,
    /// An account being renamed, and its field.
    renaming: Option<(String, Entity<InputState>)>,
    /// Removing a member removes the accounts they added too (SRV-040).
    with_accounts: bool,
    /// Empties the Add form, and gives a field the keyboard, on the next draw.
    clear_account_form: bool,
    focus_account_label: bool,
    focus_rename: bool,
}

impl ServerManager {
    pub fn new(state: Entity<AppState>, server_id: String, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let invite = new_input("", "GitHub login", false, window, cx);
        cx.subscribe_in(&invite, window, |this, _, e: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = e {
                this.invite_member(cx);
            }
            cx.notify();
        })
        .detach();
        let account_label = new_input("", "Saleem’s Max", false, window, cx);
        let account_token = new_input("", "sk-ant-oat01-…", true, window, cx);
        for field in [&account_label, &account_token] {
            cx.subscribe_in(field, window, |this, _, e: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    this.add_account(cx);
                }
                cx.notify();
            })
            .detach();
        }
        cx.observe(&state, |this, _, cx| this.follow(cx)).detach();
        let mut manager = Self {
            state,
            server_id,
            asked_connected: false,
            members: Load::Loading,
            devices: Load::Loading,
            audit: Load::Loading,
            info: Load::Loading,
            invite,
            inviting: false,
            error: None,
            confirm: None,
            replacing: false,
            key_note: None,
            all_activity: false,
            avatars: HashMap::new(),
            clear_invite: false,
            accounts: Load::Loading,
            accounts_seen: 0,
            notices_seen: 0,
            account_label,
            account_token,
            adding_account: false,
            checking: false,
            account_error: None,
            renaming: None,
            with_accounts: true,
            clear_account_form: false,
            focus_account_label: false,
            focus_rename: false,
        };
        manager.follow(cx);
        manager
    }

    fn server(&self, cx: &App) -> Option<ServerInfo> {
        self.state
            .read(cx)
            .servers
            .iter()
            .find(|s| s.id == self.server_id)
            .cloned()
    }

    /// Asks the server again whenever it comes (back) up or sends a notice
    /// (a device signed in, an account stopped working: SRV-042), and for
    /// its Claude accounts and activity whenever they change
    /// (`accounts-changed`).
    fn follow(&mut self, cx: &mut Context<'_, Self>) {
        let connected = self.server(cx).is_some_and(|s| s.state == ServerState::Connected);
        let state = self.state.read(cx);
        let now = Seen {
            accounts: state.accounts_changed.get(&self.server_id).copied().unwrap_or(0),
            notices: state.server_notices.get(&self.server_id).copied().unwrap_or(0),
        };
        let seen = Seen {
            accounts: self.accounts_seen,
            notices: self.notices_seen,
        };
        match refresh(self.asked_connected, connected, seen, now) {
            Refresh::All => self.reload(cx),
            Refresh::Accounts => {
                self.reload_accounts(cx);
                self.reload_audit(cx);
            }
            Refresh::Nothing => {}
        }
        if connected {
            (self.accounts_seen, self.notices_seen) = (now.accounts, now.notices);
        }
        self.asked_connected = connected;
    }

    /// The activity alone.
    fn reload_audit(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let audit = nodus.servers().audit(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.audit = loaded(audit);
                cx.notify();
            });
        })
        .detach();
    }

    /// Members, devices, the audit and the server, each on its own.
    fn reload(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let servers = nodus.servers();
            let members = servers.members(&id).await;
            let _ = this.update(cx, |m, cx| {
                if let Ok(members) = &members {
                    m.fetch_avatars(members, cx);
                }
                m.members = loaded(members);
                cx.notify();
            });
            let devices = servers.devices(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.devices = loaded(devices);
                cx.notify();
            });
            let accounts = servers.accounts(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.accounts = loaded(accounts);
                cx.notify();
            });
            let info = servers.info(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.info = loaded(info);
                cx.notify();
            });
            let audit = servers.audit(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.audit = loaded(audit);
                cx.notify();
            });
        })
        .detach();
    }

    fn fetch_avatars(&mut self, members: &[Member], cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        for url in members.iter().filter_map(|m| m.avatar_url.clone()) {
            if self.avatars.contains_key(&url) {
                continue;
            }
            self.avatars.insert(url.clone(), None);
            let nodus = nodus.clone();
            cx.spawn(async move |this, cx| {
                let file = nodus.servers().avatar(&url).await;
                let _ = this.update(cx, |m, cx| {
                    m.avatars.insert(url, file);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// SRV-044: invites a GitHub login.
    fn invite_member(&mut self, cx: &mut Context<'_, Self>) {
        let login = self.invite.read(cx).value().trim().to_owned();
        if login.is_empty() || self.inviting {
            return;
        }
        self.inviting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let invited = nodus.servers().invite(&id, &login).await;
            let _ = this.update(cx, |m, cx| {
                m.inviting = false;
                match invited {
                    Ok(_) => {
                        m.clear_invite = true;
                        m.reload(cx);
                    }
                    Err(err) => m.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// SRV-040, SRV-042: removes a member or revokes a device, once
    /// confirmed. One that signs this device out leaves Settings showing the
    /// server gone.
    fn confirmed(&mut self, cx: &mut Context<'_, Self>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        self.error = None;
        self.account_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        let state = self.state.clone();
        // SRV-040: the accounts they added go too, when that's ticked.
        let with_accounts = matches!(confirm, Confirm::Member(github_id) if self.accounts_added_by(github_id) > 0)
            && self.with_accounts;
        cx.spawn(async move |this, cx| {
            let done = match &confirm {
                Confirm::Member(github_id) => nodus.servers().remove_member(&id, *github_id, with_accounts).await,
                Confirm::Device(device) => nodus.servers().revoke_device(&id, device).await,
                Confirm::Account(account) => nodus.servers().remove_account(&id, account).await,
            };
            let gone = nodus.servers().status(&id).is_none();
            if gone {
                state.update(cx, |s, cx| {
                    s.refresh_servers(cx);
                    s.refresh_projects(cx).detach();
                });
                return;
            }
            let _ = this.update(cx, |m, cx| {
                match (done, &confirm) {
                    (Ok(()), _) => m.reload(cx),
                    (Err(err), Confirm::Account(_)) => m.account_error = Some(err.message),
                    (Err(err), _) => m.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// SRV-063: a `.pem` file from GitHub's App page, then `replace_key`.
    fn replace_key(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.replacing {
            return;
        }
        let stub = cx.try_global::<PickFileStub>().map(|s| s.0.clone());
        let picker = stub.is_none().then(|| {
            cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Use This Key".into()),
            })
        });
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let path = match (stub, picker) {
                (Some(path), _) => path,
                (None, Some(picker)) => match picker.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    _ => None,
                },
                (None, None) => None,
            };
            let Some(path) = path else {
                return;
            };
            let _ = this.update(cx, |m, cx| {
                m.replacing = true;
                m.key_note = None;
                cx.notify();
            });
            let replaced = match std::fs::read_to_string(&path) {
                Ok(pem) => nodus.servers().replace_key(&id, &pem).await.map_err(|err| err.message),
                Err(err) => Err(format!("Can’t read {}: {err}", path.display())),
            };
            let _ = this.update(cx, |m, cx| {
                m.replacing = false;
                m.key_note = Some(match replaced {
                    Ok(_) => (
                        true,
                        "The new key is in use. Delete the old one on the App’s page on GitHub.".to_owned(),
                    ),
                    Err(message) => (false, message),
                });
                m.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

/// How many `accounts-changed` events and notices a server has sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    accounts: u64,
    notices: u64,
}

/// What Settings asks a server again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refresh {
    Nothing,
    /// Its Claude accounts and activity.
    Accounts,
    /// Members, devices, accounts, the server and its activity.
    All,
}

/// What to ask again, from whether it was and is connected and what it has
/// sent since last asked.
fn refresh(was_connected: bool, connected: bool, seen: Seen, now: Seen) -> Refresh {
    if !connected {
        Refresh::Nothing
    } else if !was_connected || now.notices != seen.notices {
        Refresh::All
    } else if now.accounts != seen.accounts {
        Refresh::Accounts
    } else {
        Refresh::Nothing
    }
}

fn loaded<T>(answer: Result<T, nodus_core::CommandError>) -> Load<T> {
    match answer {
        Ok(value) => Load::Loaded(value),
        Err(err) => Load::Failed(err.message),
    }
}

impl Render for ServerManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        if self.clear_invite {
            self.clear_invite = false;
            self.invite.update(cx, |f, cx| f.set_value("", window, cx));
        }
        if self.clear_account_form {
            self.clear_account_form = false;
            for field in [&self.account_label, &self.account_token] {
                field.update(cx, |f, cx| f.set_value("", window, cx));
            }
        }
        if self.focus_account_label {
            self.focus_account_label = false;
            crate::components::text_field::focus_all(&self.account_label, window, cx);
        }
        if self.focus_rename {
            self.focus_rename = false;
            if let Some((_, field)) = &self.renaming {
                crate::components::text_field::focus_all(field, window, cx);
            }
        }
        let Some(server) = self.server(cx) else {
            return div().into_any_element();
        };
        if server.state != ServerState::Connected {
            return div().into_any_element();
        }
        let mut body = div().flex().flex_col();
        if let Load::Loaded(info) = &self.info
            && !info.tailscale.tagged
        {
            body = body.child(tailscale_warning(info, t));
        }
        body = body
            .child(self.members_group(&server, window, cx, t))
            .child(self.devices_group(cx, t))
            .child(self.accounts_group(window, cx, t))
            .child(self.github_group(cx, t))
            .child(self.activity_group(cx, t));
        body.into_any_element()
    }
}

impl ServerManager {
    /// UI-323: the members, you among them, and invite.
    fn members_group(&self, server: &ServerInfo, window: &Window, cx: &mut Context<'_, Self>, t: Tokens) -> gpui::Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        match &self.members {
            Load::Loading => rows.push(waiting_row("Loading members…", t)),
            Load::Failed(error) => rows.push(failed_row(error, t)),
            Load::Loaded(members) => {
                let only = members.len() <= 1;
                for member in members {
                    let you = server.github_id == Some(member.github_id);
                    let confirming = self.confirm == Some(Confirm::Member(member.github_id));
                    let file = member
                        .avatar_url
                        .as_ref()
                        .and_then(|u| self.avatars.get(u).cloned().flatten());
                    let who = div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .min_w_0()
                        .child(avatar(&member.login, file, 28., t))
                        .child(row_text(
                            member.name.clone().unwrap_or_else(|| member.login.clone()),
                            Some(if you {
                                format!("@{} · You", member.login)
                            } else {
                                format!("@{}", member.login)
                            }),
                            t,
                        ));
                    let row = if confirming {
                        let question = if you {
                            "Remove yourself? This device is signed out too.".to_owned()
                        } else {
                            format!("Remove @{}? Their devices are signed out.", member.login)
                        };
                        // SRV-040: and the Claude accounts they added, if any.
                        let theirs = self.accounts_added_by(member.github_id);
                        div()
                            .flex()
                            .flex_col()
                            .child(self.confirm_row(question, "Remove", cx, t))
                            .when(theirs > 0, |d| d.child(self.with_accounts_row(theirs, you, cx, t)))
                            .into_any_element()
                    } else {
                        let this = cx.entity();
                        let github_id = member.github_id;
                        plain_row(
                            who,
                            // The last member can't remove themselves (SRV-040).
                            (!(you && only)).then(|| {
                                button(SharedString::from(format!("server-member-remove-{github_id}")))
                                    .label("Remove")
                                    .on_click(move |_, _, cx| {
                                        this.update(cx, |m, cx| {
                                            m.confirm = Some(Confirm::Member(github_id));
                                            m.with_accounts = true;
                                            cx.notify();
                                        });
                                    })
                                    .into_any_element()
                            }),
                        )
                        .into_any_element()
                    };
                    rows.push(row);
                }
            }
        }
        let typed = !self.invite.read(cx).value().trim().is_empty();
        let this = cx.entity();
        rows.push(
            plain_row(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(crate::servers::field(&self.invite, window, cx, t)),
                Some(
                    button(SharedString::from(format!("server-invite-{}", self.server_id)))
                        .label(if self.inviting { "Inviting…" } else { "Invite" })
                        .disabled(!typed || self.inviting)
                        .on_click(move |_, _, cx| this.update(cx, Self::invite_member))
                        .into_any_element(),
                ),
            )
            .into_any_element(),
        );
        let mut d = group(Some("Members"), rows, false, t);
        if let Some(error) = &self.error {
            d = d.child(note_line(error.clone(), true, t));
        } else {
            d = d.child(note_line(
                "Invite people by their GitHub login. They connect with this server’s address and sign in.".into(),
                false,
                t,
            ));
        }
        d
    }

    /// UI-324: every member's devices, this one marked, with Revoke.
    fn devices_group(&self, cx: &mut Context<'_, Self>, t: Tokens) -> gpui::Div {
        let now = crate::parity::now_ms(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        match &self.devices {
            Load::Loading => rows.push(waiting_row("Loading devices…", t)),
            Load::Failed(error) => rows.push(failed_row(error, t)),
            Load::Loaded(devices) if devices.is_empty() => rows.push(waiting_row("No devices have signed in.", t)),
            Load::Loaded(devices) => {
                for device in devices {
                    let confirming = self.confirm == Some(Confirm::Device(device.id.clone()));
                    if confirming {
                        let question = if device.current {
                            "Revoke this device? It’s signed out, and the server’s projects leave your grid.".to_owned()
                        } else {
                            format!("Revoke {}? It’s signed out at once.", device.name)
                        };
                        rows.push(self.confirm_row(question, "Revoke", cx, t));
                        continue;
                    }
                    let seen = crate::board::cards::present::time_ago(device.last_seen_at, now);
                    let hint = if device.current {
                        format!("@{} · This device", device.login)
                    } else {
                        format!("@{} · Last seen {seen}", device.login)
                    };
                    let this = cx.entity();
                    let id = device.id.clone();
                    rows.push(
                        plain_row(
                            row_text(device.name.clone(), Some(hint), t),
                            Some(
                                button(SharedString::from(format!("server-device-revoke-{}", device.id)))
                                    .label("Revoke")
                                    .on_click(move |_, _, cx| {
                                        let id = id.clone();
                                        this.update(cx, |m, cx| {
                                            m.confirm = Some(Confirm::Device(id));
                                            cx.notify();
                                        });
                                    })
                                    .into_any_element(),
                            ),
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        group(Some("Devices"), rows, false, t)
    }

    /// UI-325: the App, where it's installed, and its key.
    fn github_group(&self, cx: &mut Context<'_, Self>, t: Tokens) -> gpui::Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        let id = self.server_id.clone();
        match &self.info {
            Load::Loading => rows.push(waiting_row("Asking GitHub…", t)),
            Load::Failed(error) => rows.push(failed_row(error, t)),
            Load::Loaded(ServerDetails { github: None, .. }) => {
                rows.push(waiting_row("This server has no GitHub App yet.", t));
            }
            Load::Loaded(info @ ServerDetails { github: Some(app), .. }) => {
                let page = app.html_url.clone();
                let owner = app.owner.clone().unwrap_or_else(|| "its owner".into());
                let workflows = if app.workflows {
                    "Sessions may change workflows"
                } else {
                    "Sessions can’t change workflows"
                };
                rows.push(
                    plain_row(
                        row_text(app.name.clone(), Some(format!("Owned by {owner} · {workflows}")), t),
                        Some(
                            button(SharedString::from(format!("server-app-page-{id}")))
                                .label("Open on GitHub")
                                .on_click(move |_, _, cx| open_url(&page, cx))
                                .into_any_element(),
                        ),
                    )
                    .into_any_element(),
                );
                if let Some(error) = &info.installations_error {
                    rows.push(failed_row(error, t));
                } else if info.installations.is_empty() {
                    rows.push(waiting_row("Not installed on any repositories yet.", t));
                }
                for installation in &info.installations {
                    rows.push(installation_row(installation, t));
                }
                let install = install_url(app);
                rows.push(
                    plain_row(
                        row_text(
                            "Install on more repositories".into(),
                            Some(
                                "Only ones you’d trust every session with, never ones that take outside contributions."
                                    .into(),
                            ),
                            t,
                        ),
                        Some(
                            button(SharedString::from(format!("server-install-more-{id}")))
                                .label("Install…")
                                .on_click(move |_, _, cx| open_url(&install, cx))
                                .into_any_element(),
                        ),
                    )
                    .into_any_element(),
                );
                let this = cx.entity();
                rows.push(
                    plain_row(
                        row_text(
                            "App key".into(),
                            Some("Generate a new private key on the App’s page, then pick the .pem file GitHub downloads.".into()),
                            t,
                        ),
                        Some(
                            button(SharedString::from(format!("server-replace-key-{id}")))
                                .label(if self.replacing { "Replacing…" } else { "Replace App Key…" })
                                .disabled(self.replacing)
                                .on_click(move |_, window, cx| this.update(cx, |m, cx| m.replace_key(window, cx)))
                                .into_any_element(),
                        ),
                    )
                    .into_any_element(),
                );
            }
        }
        let mut d = group(Some("GitHub"), rows, false, t);
        if let Some((ok, message)) = &self.key_note {
            d = d.child(note_line(message.clone(), !ok, t));
        }
        d
    }

    /// UI-326: the audit, newest first.
    fn activity_group(&self, cx: &mut Context<'_, Self>, t: Tokens) -> gpui::Div {
        let now = crate::parity::now_ms(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut more = 0;
        match &self.audit {
            Load::Loading => rows.push(waiting_row("Loading…", t)),
            Load::Failed(error) => rows.push(failed_row(error, t)),
            Load::Loaded(lines) => {
                let shown = if self.all_activity { lines.len() } else { ACTIVITY_SHOWN };
                more = lines.len().saturating_sub(shown);
                for line in lines.iter().take(shown) {
                    rows.push(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .px(px(14.))
                            .py(px(8.))
                            .text_size(TEXT_SM)
                            .child(div().min_w_0().truncate().child(describe(line)))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(t.text_tertiary)
                                    .child(crate::board::cards::present::time_ago(line.time, now)),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        let mut d = group(Some("Recent activity"), rows, false, t);
        if more > 0 {
            let this = cx.entity();
            d = d.child(
                div()
                    .id(SharedString::from(format!("server-activity-all-{}", self.server_id)))
                    .mt(px(8.))
                    .px(px(2.))
                    .text_size(TEXT_SM)
                    .text_color(t.accent)
                    .hover(Styled::underline)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |m, cx| {
                            m.all_activity = true;
                            cx.notify();
                        });
                    })
                    .child(format!("Show {more} more")),
            );
        }
        d
    }

    /// A row that asks before removing or revoking, in its place.
    fn confirm_row(&self, question: String, verb: &'static str, cx: &mut Context<'_, Self>, t: Tokens) -> AnyElement {
        let (cancel, go) = (cx.entity(), cx.entity());
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(14.))
            .py(px(9.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(TEXT_SM)
                    .text_color(t.text_primary)
                    .child(question),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(8.))
                    .child(
                        button("server-confirm-cancel")
                            .label("Cancel")
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |m, cx| {
                                    m.confirm = None;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        button("server-confirm")
                            .kind(Kind::Danger)
                            .label(verb)
                            .on_click(move |_, _, cx| go.update(cx, Self::confirmed)),
                    ),
            )
            .into_any_element()
    }
}

/// SRV-029, UI-327: the server's machine doesn't carry `tag:nodus`.
fn tailscale_warning(info: &ServerDetails, t: Tokens) -> AnyElement {
    let text = if info.tailscale.running {
        "This server’s machine isn’t tagged tag:nodus on its tailnet, so the access rules don’t keep sessions away from members’ computers. Tag it as the README says."
    } else {
        "Tailscale isn’t running on this server, so it isn’t tagged tag:nodus and the tailnet’s access rules don’t fence it in."
    };
    div()
        .mt(px(16.))
        .flex()
        .gap(px(10.))
        .px(px(14.))
        .py(px(11.))
        .rounded(RADIUS_MD)
        .border(px(0.5))
        .border_color(t.status_waiting)
        .bg(t.bg_sunken)
        .child(
            div()
                .pt(px(1.))
                .child(icon(Icon::CircleAlert, 14., 2., t.status_waiting)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().font_weight(FontWeight::MEDIUM).child("Not tagged on the tailnet"))
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.text_secondary)
                        .child(text),
                ),
        )
        .into_any_element()
}

fn installation_row(installation: &Installation, t: Tokens) -> AnyElement {
    let repos = if installation.repository_selection == "all" {
        "All repositories"
    } else {
        "Selected repositories"
    };
    let page = installation.html_url.clone();
    plain_row(
        row_text(format!("Installed on {}", installation.account), Some(repos.into()), t),
        Some(
            button(SharedString::from(format!("server-installation-{}", installation.id)))
                .ghost()
                .label("Configure…")
                .on_click(move |_, _, cx| open_url(&page, cx))
                .into_any_element(),
        ),
    )
    .into_any_element()
}

fn waiting_row(text: &str, t: Tokens) -> AnyElement {
    div()
        .px(px(14.))
        .py(px(11.))
        .text_size(TEXT_SM)
        .text_color(t.text_secondary)
        .child(text.to_owned())
        .into_any_element()
}

fn failed_row(error: &str, t: Tokens) -> AnyElement {
    div()
        .px(px(14.))
        .py(px(11.))
        .text_size(TEXT_SM)
        .text_color(t.status_waiting)
        .child(error.to_owned())
        .into_any_element()
}

/// A line under a group: a hint, or what went wrong.
fn note_line(text: String, failed: bool, t: Tokens) -> gpui::Div {
    div()
        .mt(px(8.))
        .px(px(2.))
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .text_color(if failed { t.status_waiting } else { t.text_secondary })
        .child(text)
}

/// An audit line in words (SRV-210).
fn describe(line: &AuditLine) -> String {
    let who = line.member.as_ref().map(|m| format!("@{}", m.login));
    let detail = |key: &str| line.details.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    let login = detail("login").map(|l| format!("@{l}"));
    let someone = || who.clone().unwrap_or_else(|| "Someone".into());
    match line.event.as_str() {
        "setup.claimed" => "The setup code was claimed".into(),
        "server.named" => match (who, detail("name")) {
            (Some(who), Some(name)) => format!("{who} named the server {name}"),
            (None, Some(name)) => format!("The server was named {name}"),
            (_, None) => "The server was named".into(),
        },
        "github.app-created" => "The GitHub App was created".into(),
        "github.installations-changed" => "The GitHub App’s installations changed".into(),
        "github.key-replaced" => format!("{} replaced the App’s key", someone()),
        "member.added" => format!("{} became the first member", login.unwrap_or_else(someone)),
        "member.signed-in" => match (who, detail("device")) {
            (Some(who), Some(device)) => format!("{who} signed in on {device}"),
            (who, _) => format!("{} signed in", who.unwrap_or_else(|| "Someone".into())),
        },
        "member.refused" => format!(
            "{} wasn’t let in: not a member",
            login.unwrap_or_else(|| "Someone".into())
        ),
        "member.invited" => format!("{} invited {}", someone(), login.unwrap_or_default()),
        "member.removed" => format!("{} removed {}", someone(), login.unwrap_or_default()),
        "device.revoked" => format!(
            "{} revoked {}",
            someone(),
            detail("name").unwrap_or_else(|| "a device".into())
        ),
        // SRV-210: the pool's.
        "account.added" => format!(
            "{} added the Claude account {}",
            someone(),
            detail("label").unwrap_or_default()
        ),
        "account.renamed" => format!(
            "{} renamed the Claude account {} to {}",
            someone(),
            detail("was").unwrap_or_default(),
            detail("label").unwrap_or_default()
        ),
        "account.removed" => format!(
            "{} removed the Claude account {}",
            someone(),
            detail("label").unwrap_or_default()
        ),
        // SRV-200
        "server.updated" => format!(
            "{} updated the server from Nodus {} to {}",
            someone(),
            detail("from").unwrap_or_default(),
            detail("to").unwrap_or_default()
        ),
        "server.up-to-date" => format!(
            "{} checked for a server update: Nodus {} is the newest",
            someone(),
            detail("version").unwrap_or_default()
        ),
        "server.update-failed" => format!(
            "{} couldn’t update the server: {}",
            someone(),
            detail("error").unwrap_or_default()
        ),
        "account.failed" => format!(
            "{}’s Claude account stopped working",
            detail("label").unwrap_or_else(|| "A".into())
        ),
        other => {
            let words = other.replace(['.', '-'], " ");
            match who {
                Some(who) => format!("{who}: {words}"),
                None => words,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodus_core::hosted::{AuditDevice, AuditMember};
    use serde_json::json;

    fn line(event: &str, member: Option<&str>, details: serde_json::Value) -> AuditLine {
        AuditLine {
            time: 1.0,
            event: event.into(),
            member: member.map(|login| AuditMember {
                github_id: 1,
                login: login.into(),
            }),
            device: member.map(|_| AuditDevice {
                id: "d".into(),
                name: "Studio".into(),
            }),
            details,
        }
    }

    /// Settings → Servers asks again when a notice or an event says so.
    #[test]
    fn asks_again_when_the_server_says_something_changed() {
        let seen = |accounts, notices| Seen { accounts, notices };
        // Coming (back) up: everything.
        assert_eq!(refresh(false, true, seen(0, 0), seen(0, 0)), Refresh::All);
        assert_eq!(refresh(true, true, seen(1, 2), seen(1, 2)), Refresh::Nothing);
        // A device signed in, or an account stopped working: everything.
        assert_eq!(refresh(true, true, seen(1, 2), seen(1, 3)), Refresh::All);
        assert_eq!(refresh(true, true, seen(1, 2), seen(2, 3)), Refresh::All);
        // The pool changed: its accounts and the activity.
        assert_eq!(refresh(true, true, seen(1, 2), seen(2, 2)), Refresh::Accounts);
        // Away, nothing is asked.
        assert_eq!(refresh(true, false, seen(1, 2), seen(5, 5)), Refresh::Nothing);
    }

    #[test]
    fn says_what_happened_in_words() {
        let cases = [
            (line("setup.claimed", None, json!({})), "The setup code was claimed"),
            (
                line("member.signed-in", Some("octo"), json!({ "device": "Laptop" })),
                "@octo signed in on Laptop",
            ),
            (
                line("member.invited", Some("octo"), json!({ "login": "alice" })),
                "@octo invited @alice",
            ),
            (
                line("member.refused", None, json!({ "login": "mallory" })),
                "@mallory wasn’t let in: not a member",
            ),
            (
                line("device.revoked", Some("octo"), json!({ "name": "Laptop" })),
                "@octo revoked Laptop",
            ),
            (
                line("server.named", Some("octo"), json!({ "name": "Box" })),
                "@octo named the server Box",
            ),
            (
                line("server.named", None, json!({ "name": "Box" })),
                "The server was named Box",
            ),
            (line("server.restarted", None, json!({})), "server restarted"),
            // SRV-200
            (
                line(
                    "server.updated",
                    Some("octo"),
                    json!({ "from": "1.0.12", "to": "1.1.0" }),
                ),
                "@octo updated the server from Nodus 1.0.12 to 1.1.0",
            ),
            (
                line("server.up-to-date", Some("octo"), json!({ "version": "1.1.0" })),
                "@octo checked for a server update: Nodus 1.1.0 is the newest",
            ),
            (
                line(
                    "server.update-failed",
                    Some("octo"),
                    json!({ "error": "The update’s signature doesn’t match Nodus’s signing key. Nothing was installed." }),
                ),
                "@octo couldn’t update the server: The update’s signature doesn’t match Nodus’s signing key. Nothing was installed.",
            ),
            (
                line("account.added", Some("octo"), json!({ "label": "Team Max" })),
                "@octo added the Claude account Team Max",
            ),
            (
                line(
                    "account.failed",
                    None,
                    json!({ "label": "Team Max", "error": "billing_error" }),
                ),
                "Team Max’s Claude account stopped working",
            ),
        ];
        for (line, words) in cases {
            assert_eq!(describe(&line), words);
        }
    }
}
