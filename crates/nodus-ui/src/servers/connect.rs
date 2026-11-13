//! **Connect to Server…** (SRV-060…063, SRV-041, UI-316…UI-322): the
//! address first; then, by its hello, the setup code and the wizard for a
//! server that waits to be set up, or **Sign in with GitHub** for one that
//! is. **Use a token instead** keeps the development token's form.
//!
//! The wizard's steps resume from `setup:status` when the dialog opens
//! again: the core keeps the setup session for this run, and a server
//! signed in already picks up at Members.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_core::hosted::{GithubApp, Member, SETUP_EXPIRED, SetupStatus, SetupStep, SignInStarted, install_url};
use nodus_core::{SetupSession, SignIn};

use super::{ServersDialog, avatar, field, heading_owned, label, note, open_url};
use crate::components::button::button;
use crate::components::icon::{Icon, icon};
use crate::state::{AppState, ServerDialog};
use crate::theme::fonts;
use crate::theme::tokens::*;

/// How often the wizard asks whether the App has been made, and whether
/// it's been installed.
const APP_POLL: Duration = Duration::from_secs(2);
const INSTALL_POLL: Duration = Duration::from_secs(3);

/// Where the connect dialog is.
pub(super) enum Step {
    /// The address, which hello sorts out (UI-316).
    Address,
    /// The address and a development token (UI-309).
    Token,
    /// `Set up <name>`: the setup code (UI-317).
    Code { url: String, name: String },
    /// Wizard step 1, the name (UI-318).
    Name { session: SetupSession },
    /// Wizard step 2, the GitHub App (UI-319).
    Github { session: SetupSession, phase: Phase },
    /// Sign in with GitHub (UI-320): the wizard's, with its setup session,
    /// or a server's set up already.
    SignIn {
        url: String,
        name: String,
        setup: Option<SetupSession>,
        started: Option<SignInStarted>,
        /// Why the last try ended, in the server's words.
        ended: Option<String>,
    },
    /// Wizard step 3, inviting members (UI-321).
    Members { server_id: String, name: String },
    /// Wizard step 4, Claude accounts (UI-333).
    Accounts { server_id: String, name: String },
    /// Wizard step 5, the first project (UI-335).
    Project { server_id: String, name: String },
    /// Set up (UI-322).
    Done { server_id: String, name: String },
}

/// Where making the App is.
pub(super) enum Phase {
    /// Personal or an organization, workflows, and Create GitHub App.
    Create,
    /// GitHub's page is open in the browser; waiting for the App.
    Making,
    /// Made; waiting until it's installed somewhere.
    Install { app: GithubApp },
}

impl Step {
    /// Its place among the wizard's steps, counting from 0; none outside it.
    fn wizard_index(&self) -> Option<usize> {
        match self {
            Self::Name { .. } => Some(0),
            // Signing in finishes step 2 (SRV-061).
            Self::Github { .. } | Self::SignIn { setup: Some(_), .. } => Some(1),
            Self::Members { .. } => Some(2),
            Self::Accounts { .. } => Some(3),
            Self::Project { .. } => Some(4),
            Self::Done { .. } => Some(5),
            _ => None,
        }
    }
}

/// The wizard's steps, as its header names them: SRV-061's five.
const WIZARD: [&str; 5] = ["Name", "GitHub", "Members", "Claude", "Project"];

impl ServersDialog {
    /// Goes to `step`: fields it shows get the keyboard, the error goes,
    /// and polls of the step before stop.
    pub(super) fn go(&mut self, step: Step, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.poll += 1;
        self.error = None;
        self.connecting = false;
        self.copied = false;
        let field = match &step {
            Step::Address => Some(&self.address),
            Step::Token if self.address.read(cx).value().trim().is_empty() => Some(&self.address),
            Step::Token => Some(&self.token),
            Step::Code { .. } => Some(&self.code),
            Step::Name { .. } => Some(&self.name),
            Step::Members { .. } => Some(&self.invite),
            Step::Accounts { .. } => Some(&self.account_label),
            Step::Project { .. } => Some(&self.search),
            _ => None,
        };
        match field {
            Some(field) => super::focus_all(field, window, cx),
            None => window.focus(&self.panel, cx),
        }
        self.step = step;
        match &self.step {
            Step::Github {
                phase: Phase::Making, ..
            } => self.poll_app(cx),
            Step::Github {
                phase: Phase::Install { .. },
                ..
            } => self.poll_installed(cx),
            Step::SignIn { started: None, .. } => self.start_sign_in(cx),
            Step::Members { server_id, .. } => {
                let server_id = server_id.clone();
                self.load_members(server_id, cx);
            }
            Step::Accounts { server_id, .. } => {
                let server_id = server_id.clone();
                self.load_accounts(server_id, cx);
            }
            Step::Project { server_id, .. } => {
                let server_id = server_id.clone();
                self.add_error = None;
                self.repos.remove(&server_id);
                self.load_repos(&server_id, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// Enter, or the step's main button.
    pub(super) fn primary(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.connecting {
            return;
        }
        match &self.step {
            Step::Address => self.check_address(window, cx),
            Step::Token => self.connect(window, cx),
            Step::Code { url, .. } => {
                let url = url.clone();
                self.claim(url, window, cx);
            }
            Step::Name { session } => {
                let session = session.clone();
                self.save_name(session, window, cx);
            }
            Step::Github {
                session,
                phase: Phase::Create,
            } => {
                let session = session.clone();
                self.make_app(session, window, cx);
            }
            Step::Github {
                phase: Phase::Install { app },
                ..
            } => open_url(&install_url(app), cx),
            Step::SignIn {
                started: Some(started), ..
            } => open_url(&started.verification_uri, cx),
            Step::SignIn { ended: Some(_), .. } => self.retry_sign_in(window, cx),
            Step::Members { .. } if !self.invite.read(cx).value().trim().is_empty() => self.invite_member(cx),
            Step::Members { server_id, .. } => {
                let server_id = server_id.clone();
                self.finish_members(server_id, window, cx);
            }
            Step::Accounts { .. } if self.account_typed(cx) => self.add_account(cx),
            Step::Accounts { server_id, .. } => {
                let server_id = server_id.clone();
                self.finish_accounts(server_id, window, cx);
            }
            // Enter adds the one repository that matches what's typed.
            Step::Project { server_id, .. } => {
                let server_id = server_id.clone();
                if let Some(full_name) = self.only_match(&server_id, cx) {
                    self.add_first_project(full_name, window, cx);
                }
            }
            Step::Done { server_id, .. } => {
                let server_id = server_id.clone();
                self.state.update(cx, |s, cx| {
                    s.refresh_servers(cx);
                    s.open_server_dialog(
                        ServerDialog::Projects {
                            server_id: Some(server_id),
                        },
                        cx,
                    );
                });
                self.serial += 1;
            }
            Step::Github { .. } | Step::SignIn { .. } => {}
        }
    }

    /// The fields Tab goes between on this step.
    pub(super) fn fields(&self) -> Vec<gpui::Entity<gpui::component::input::InputState>> {
        match &self.step {
            Step::Address => vec![self.address.clone()],
            Step::Token => vec![self.address.clone(), self.token.clone()],
            Step::Code { .. } => vec![self.code.clone()],
            Step::Name { .. } => vec![self.name.clone()],
            Step::Github {
                phase: Phase::Create, ..
            } if self.for_org => vec![self.org.clone()],
            Step::Members { .. } => vec![self.invite.clone()],
            Step::Accounts { .. } => vec![self.account_label.clone(), self.account_token.clone()],
            Step::Project { .. } => vec![self.search.clone()],
            _ => Vec::new(),
        }
    }

    // ---- what each step does

    /// UI-316: hello decides. A server waiting to be set up asks for its
    /// code, or resumes the setup session of this run; one set up already
    /// signs in, or resumes Members when this device made it.
    fn check_address(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let url = self.address.read(cx).value().trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.connecting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let known: Vec<_> = self.state.read(cx).servers.clone();
        cx.spawn_in(window, async move |this, cx| {
            let hello = nodus.servers().hello(&url).await;
            let hello = match hello {
                Ok(hello) => hello,
                Err(err) => {
                    let _ = this.update(cx, |d, cx| d.failed(err.message, cx));
                    return;
                }
            };
            if hello.needs_setup {
                let resumed = match nodus.servers().setup_session(&url) {
                    Some(session) => match session.status().await {
                        Ok(status) => Some((session, status)),
                        Err(_) => {
                            nodus.servers().forget_setup(&url);
                            None
                        }
                    },
                    None => None,
                };
                let _ = this.update_in(cx, |d, window, cx| match resumed {
                    Some((session, status)) => d.resume(session, status, window, cx),
                    None => {
                        let name = hello.name.clone();
                        d.go(Step::Code { url, name }, window, cx);
                    }
                });
                return;
            }
            // Signed in here already, mid-wizard: Members again.
            let here = known
                .iter()
                .find(|s| same_address(&s.url, &url) && s.login.is_some())
                .cloned();
            let members = match &here {
                Some(server) => nodus.servers().setup_status(&server.id).await.ok().filter(|status| {
                    matches!(
                        status.step,
                        SetupStep::Members | SetupStep::Accounts | SetupStep::Project
                    )
                }),
                None => None,
            };
            let _ = this.update_in(cx, |d, window, cx| match (here, members) {
                (Some(server), Some(status)) if status.step == SetupStep::Project => d.go(
                    Step::Project {
                        server_id: server.id,
                        name: hello.name,
                    },
                    window,
                    cx,
                ),
                (Some(server), Some(status)) if status.step == SetupStep::Accounts => d.go(
                    Step::Accounts {
                        server_id: server.id,
                        name: hello.name,
                    },
                    window,
                    cx,
                ),
                (Some(server), Some(_)) => d.go(
                    Step::Members {
                        server_id: server.id,
                        name: hello.name,
                    },
                    window,
                    cx,
                ),
                _ => d.go(
                    Step::SignIn {
                        url,
                        name: hello.name,
                        setup: None,
                        started: None,
                        ended: None,
                    },
                    window,
                    cx,
                ),
            });
        })
        .detach();
    }

    /// UI-317: claims the code, then the wizard where its server stands.
    fn claim(&mut self, url: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let code = self.code.read(cx).value().trim().to_owned();
        if code.is_empty() {
            return;
        }
        self.connecting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let claimed = match nodus.servers().claim(&url, &code).await {
                Ok(session) => session.status().await.map(|status| (session, status)),
                Err(err) => Err(err),
            };
            let _ = this.update_in(cx, |d, window, cx| match claimed {
                Ok((session, status)) => d.resume(session, status, window, cx),
                Err(err) => d.failed(err.message, cx),
            });
        })
        .detach();
    }

    /// The wizard's step as `setup:status` has it (SRV-061).
    fn resume(&mut self, session: SetupSession, status: SetupStatus, window: &mut Window, cx: &mut Context<'_, Self>) {
        // The name as the server has it, for this step and the ones after.
        let name = status.name.clone();
        self.name.update(cx, |f, cx| f.set_value(name, window, cx));
        match (status.step, status.github) {
            (SetupStep::Name, _) => self.go(Step::Name { session }, window, cx),
            (_, None) => self.go(
                Step::Github {
                    session,
                    phase: Phase::Create,
                },
                window,
                cx,
            ),
            (_, Some(app)) => self.go(
                Step::Github {
                    session,
                    phase: Phase::Install { app },
                },
                window,
                cx,
            ),
        }
    }

    /// UI-318: step 1.
    fn save_name(&mut self, session: SetupSession, window: &mut Window, cx: &mut Context<'_, Self>) {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.connecting = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let saved = session.name(&name).await;
            let _ = this.update_in(cx, |d, window, cx| match saved {
                Ok(status) => d.resume(session, status, window, cx),
                Err(err) => d.setup_failed(&session, err.message, window, cx),
            });
        })
        .detach();
    }

    /// UI-319: a nonce, then GitHub's page in the browser; the wizard waits
    /// for the App.
    fn make_app(&mut self, session: SetupSession, window: &mut Window, cx: &mut Context<'_, Self>) {
        let owner = if self.for_org {
            self.org.read(cx).value().trim().to_owned()
        } else {
            String::new()
        };
        if self.for_org && owner.is_empty() {
            return;
        }
        let workflows = self.workflows;
        self.connecting = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let nonce = session.nonce(workflows).await;
            let _ = this.update_in(cx, |d, window, cx| match nonce {
                Ok(nonce) => {
                    d.app_page = Some(session.start_url(&nonce.nonce, &owner));
                    open_url(&session.start_url(&nonce.nonce, &owner), cx);
                    d.go(
                        Step::Github {
                            session,
                            phase: Phase::Making,
                        },
                        window,
                        cx,
                    );
                }
                Err(err) => d.setup_failed(&session, err.message, window, cx),
            });
        })
        .detach();
    }

    /// Asks `setup:status` until the App is there.
    fn poll_app(&mut self, cx: &mut Context<'_, Self>) {
        let Step::Github { session, .. } = &self.step else {
            return;
        };
        let (session, poll) = (session.clone(), self.poll);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(APP_POLL).await;
                if !this.update(cx, |d, _| d.poll == poll).unwrap_or(false) {
                    return;
                }
                if let Ok(SetupStatus { github: Some(app), .. }) = session.status().await {
                    let _ = this.update(cx, |d, cx| {
                        if d.poll == poll {
                            d.poll += 1;
                            d.step = Step::Github {
                                session,
                                phase: Phase::Install { app },
                            };
                            d.poll_installed(cx);
                            cx.notify();
                        }
                    });
                    return;
                }
            }
        })
        .detach();
    }

    /// Asks `github:installations` until there's one, then signs in.
    fn poll_installed(&mut self, cx: &mut Context<'_, Self>) {
        let Step::Github { session, .. } = &self.step else {
            return;
        };
        let (session, poll) = (session.clone(), self.poll);
        let name = self.server_name(cx);
        cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                if !first {
                    cx.background_executor().timer(INSTALL_POLL).await;
                }
                first = false;
                if !this.update(cx, |d, _| d.poll == poll).unwrap_or(false) {
                    return;
                }
                let installed = session.installations().await.is_ok_and(|i| !i.is_empty());
                if installed {
                    let url = session.url().to_owned();
                    let _ = this.update(cx, |d, cx| {
                        if d.poll == poll {
                            d.poll += 1;
                            d.step = Step::SignIn {
                                url,
                                name,
                                setup: Some(session),
                                started: None,
                                ended: None,
                            };
                            d.start_sign_in(cx);
                            cx.notify();
                        }
                    });
                    return;
                }
            }
        })
        .detach();
    }

    /// UI-320: a code to type on GitHub, then polls until it's said yes.
    fn start_sign_in(&mut self, cx: &mut Context<'_, Self>) {
        let Step::SignIn { url, setup, .. } = &self.step else {
            return;
        };
        let (url, setup, poll) = (url.clone(), setup.clone(), self.poll);
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let device = nodus.servers().device_name().await;
            let started = match &setup {
                Some(session) => session.start_sign_in(&device).await,
                None => nodus.servers().start_sign_in(&url, &device, None).await,
            };
            let started = match started {
                Ok(started) => started,
                Err(err) => {
                    let _ = this.update(cx, |d, cx| d.sign_in_ended(poll, err.message, cx));
                    return;
                }
            };
            let handle = started.handle.clone();
            let mut wait = Duration::from_secs(started.interval.max(1));
            let shown = this.update(cx, |d, cx| {
                if d.poll != poll {
                    return false;
                }
                if let Step::SignIn { started: s, .. } = &mut d.step {
                    *s = Some(started);
                }
                cx.notify();
                true
            });
            if !shown.unwrap_or(false) {
                return;
            }
            loop {
                cx.background_executor().timer(wait).await;
                if !this.update(cx, |d, _| d.poll == poll).unwrap_or(false) {
                    return;
                }
                match nodus.servers().poll_sign_in(&url, &handle).await {
                    Ok(SignIn::Pending { interval }) => wait = Duration::from_secs(interval.max(1)),
                    Ok(SignIn::SignedIn { server, .. }) => {
                        let _ = this.update(cx, |d, cx| d.signed_in(poll, server, cx));
                        return;
                    }
                    Ok(SignIn::Ended { message }) | Err(nodus_core::CommandError { message, .. }) => {
                        let _ = this.update(cx, |d, cx| d.sign_in_ended(poll, message, cx));
                        return;
                    }
                }
            }
        })
        .detach();
    }

    fn sign_in_ended(&mut self, poll: u64, message: String, cx: &mut Context<'_, Self>) {
        if self.poll != poll {
            return;
        }
        if let Step::SignIn { ended, started, .. } = &mut self.step {
            *ended = Some(message);
            *started = None;
        }
        cx.notify();
    }

    fn retry_sign_in(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Step::SignIn { url, name, setup, .. } = &self.step else {
            return;
        };
        let step = Step::SignIn {
            url: url.clone(),
            name: name.clone(),
            setup: setup.clone(),
            started: None,
            ended: None,
        };
        self.go(step, window, cx);
    }

    /// Signed in: the wizard goes on to Members; otherwise the picker.
    fn signed_in(&mut self, poll: u64, server: nodus_domain::ServerInfo, cx: &mut Context<'_, Self>) {
        if self.poll != poll {
            return;
        }
        let wizard = matches!(self.step, Step::SignIn { setup: Some(_), .. });
        self.state.update(cx, AppState::refresh_servers);
        if wizard {
            self.poll += 1;
            self.step = Step::Members {
                server_id: server.id.clone(),
                name: server.name.clone(),
            };
            self.focus_invite = true;
            self.load_members(server.id, cx);
        } else {
            self.listings.remove(&server.id);
            self.state.update(cx, |s, cx| {
                s.open_server_dialog(
                    ServerDialog::Projects {
                        server_id: Some(server.id),
                    },
                    cx,
                );
            });
            self.serial += 1;
        }
        cx.notify();
    }

    fn load_members(&mut self, server_id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let members = nodus.servers().members(&server_id).await;
            let _ = this.update(cx, |d, cx| {
                match members {
                    Ok(members) => {
                        d.fetch_avatars(&members, cx);
                        d.members = members;
                    }
                    Err(err) => d.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// UI-321: invites a GitHub login.
    fn invite_member(&mut self, cx: &mut Context<'_, Self>) {
        let Step::Members { server_id, .. } = &self.step else {
            return;
        };
        let login = self.invite.read(cx).value().trim().to_owned();
        if login.is_empty() || self.connecting {
            return;
        }
        let server_id = server_id.clone();
        self.connecting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let invited = nodus.servers().invite(&server_id, &login).await;
            let _ = this.update(cx, |d, cx| {
                d.connecting = false;
                match invited {
                    Ok(member) => {
                        d.fetch_avatars(std::slice::from_ref(&member), cx);
                        d.members.push(member);
                        d.clear_invite = true;
                    }
                    Err(err) => d.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Step 3 is done or skipped: `setup:done members`, then Claude accounts.
    fn finish_members(&mut self, server_id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.connecting = true;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let name = self.server_name(cx);
        cx.spawn_in(window, async move |this, cx| {
            let done = nodus.servers().finish_step(&server_id, SetupStep::Members).await;
            let _ = this.update_in(cx, |d, window, cx| match done {
                Ok(_) => d.go(Step::Accounts { server_id, name }, window, cx),
                Err(err) => d.failed(err.message, cx),
            });
        })
        .detach();
    }

    fn load_accounts(&mut self, server_id: String, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let accounts = nodus.servers().accounts(&server_id).await;
            let _ = this.update(cx, |d, cx| {
                match accounts {
                    Ok(accounts) => d.accounts = accounts,
                    Err(err) => d.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether both of a Claude account's fields hold something.
    fn account_typed(&self, cx: &gpui::App) -> bool {
        !self.account_label.read(cx).value().trim().is_empty() && !self.account_token.read(cx).value().trim().is_empty()
    }

    /// UI-333: adds the account, once the server has tried its token.
    fn add_account(&mut self, cx: &mut Context<'_, Self>) {
        let Step::Accounts { server_id, .. } = &self.step else {
            return;
        };
        if !self.account_typed(cx) || self.connecting {
            return;
        }
        let server_id = server_id.clone();
        let label = self.account_label.read(cx).value().trim().to_owned();
        let token = self.account_token.read(cx).value().trim().to_owned();
        self.connecting = true;
        self.error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn(async move |this, cx| {
            let added = nodus.servers().add_account(&server_id, &label, &token).await;
            let _ = this.update(cx, |d, cx| {
                d.connecting = false;
                match added {
                    Ok(account) => {
                        d.accounts.push(account);
                        d.clear_account = true;
                    }
                    Err(err) => d.error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Step 4 is done or skipped: `setup:done accounts`, then the first
    /// project.
    fn finish_accounts(&mut self, server_id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.connecting = true;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let name = self.server_name(cx);
        cx.spawn_in(window, async move |this, cx| {
            let done = nodus.servers().finish_step(&server_id, SetupStep::Accounts).await;
            let _ = this.update_in(cx, |d, window, cx| match done {
                Ok(_) => d.go(Step::Project { server_id, name }, window, cx),
                Err(err) => d.failed(err.message, cx),
            });
        })
        .detach();
    }

    /// The repository Enter would add on step 5: the only one matching
    /// what's typed.
    fn only_match(&self, server_id: &str, cx: &gpui::App) -> Option<String> {
        let Some(super::Repos::Loaded(repos)) = self.repos.get(server_id) else {
            return None;
        };
        let query = self.search.read(cx).value().trim().to_owned();
        let mut shown = repos.iter().filter(|r| super::repo_matches(&r.full_name, &query));
        match (shown.next(), shown.next()) {
            (Some(only), None) => Some(only.full_name.clone()),
            _ => None,
        }
    }

    /// UI-335: clones the repository on the server as its first project
    /// (`projects:add-github`, SRV-120), which links it into the grid, then
    /// finishes step 5.
    pub(super) fn add_first_project(&mut self, full_name: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Step::Project { server_id, .. } = &self.step else {
            return;
        };
        if self.adding.is_some() || self.connecting {
            return;
        }
        let server_id = server_id.clone();
        self.adding = Some(full_name.clone());
        self.add_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let added = nodus.servers().add_from_github(&server_id, &full_name).await;
            let _ = this.update_in(cx, |d, window, cx| {
                d.adding = None;
                match added {
                    Ok(_) => {
                        d.state.update(cx, |s, cx| s.refresh_projects(cx).detach());
                        d.finish_project(server_id, window, cx);
                    }
                    Err(err) => d.add_error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Step 5 is done or skipped: `setup:done project`, then Done.
    fn finish_project(&mut self, server_id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.connecting = true;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let name = self.server_name(cx);
        cx.spawn_in(window, async move |this, cx| {
            let done = nodus.servers().finish_step(&server_id, SetupStep::Project).await;
            let _ = this.update_in(cx, |d, window, cx| match done {
                Ok(_) => d.go(Step::Done { server_id, name }, window, cx),
                Err(err) => d.failed(err.message, cx),
            });
        })
        .detach();
    }

    fn failed(&mut self, message: String, cx: &mut Context<'_, Self>) {
        self.connecting = false;
        self.error = Some(message);
        cx.notify();
    }

    /// A setup call failed: when the session's token is no good any more,
    /// back to the code; otherwise the server's words.
    fn setup_failed(
        &mut self,
        session: &SetupSession,
        message: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if message == SETUP_EXPIRED {
            let nodus = self.state.read(cx).nodus.clone();
            nodus.servers().forget_setup(session.url());
            let name = self.server_name(cx);
            self.code.update(cx, |f, cx| f.set_value("", window, cx));
            self.go(
                Step::Code {
                    url: session.url().to_owned(),
                    name,
                },
                window,
                cx,
            );
        }
        self.failed(message, cx);
    }

    /// The name the server goes by on this step.
    fn server_name(&self, cx: &gpui::App) -> String {
        match &self.step {
            Step::Code { name, .. }
            | Step::SignIn { name, .. }
            | Step::Members { name, .. }
            | Step::Accounts { name, .. }
            | Step::Project { name, .. }
            | Step::Done { name, .. } => name.clone(),
            _ => {
                let typed = self.name.read(cx).value().trim().to_owned();
                if typed.is_empty() { "this server".into() } else { typed }
            }
        }
    }

    // ---- drawing

    pub(super) fn render_step(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        if self.clear_invite {
            self.clear_invite = false;
            self.invite.update(cx, |f, cx| f.set_value("", window, cx));
        }
        if self.clear_account {
            self.clear_account = false;
            for field in [&self.account_label, &self.account_token] {
                field.update(cx, |f, cx| f.set_value("", window, cx));
            }
            super::focus_all(&self.account_label, window, cx);
        }
        if self.focus_invite {
            self.focus_invite = false;
            super::focus_all(&self.invite, window, cx);
        }
        let wizard = self.step.wizard_index();
        let body = match &self.step {
            Step::Address => self.render_address(t, window, cx),
            Step::Token => self.render_connect(t, window, cx),
            Step::Code { name, .. } => {
                let name = name.clone();
                self.render_code(&name, t, window, cx)
            }
            Step::Name { .. } => self.render_name(t, window, cx),
            Step::Github { phase, .. } => match phase {
                Phase::Create => self.render_create(t, window, cx),
                Phase::Making => self.render_making(t, cx),
                Phase::Install { app } => {
                    let app = app.clone();
                    self.render_install(&app, t, cx)
                }
            },
            Step::SignIn { .. } => self.render_sign_in(t, cx),
            Step::Members { name, .. } => {
                let name = name.clone();
                self.render_members(&name, t, window, cx)
            }
            Step::Accounts { name, .. } => {
                let name = name.clone();
                self.render_accounts(&name, t, window, cx)
            }
            Step::Project { server_id, name } => {
                let (server_id, name) = (server_id.clone(), name.clone());
                self.render_project(&server_id, &name, t, window, cx)
            }
            Step::Done { name, .. } => {
                let name = name.clone();
                self.render_done(&name, t, cx)
            }
        };
        div()
            .flex()
            .flex_col()
            .pt(px(22.))
            .px(px(22.))
            .pb(px(18.))
            .when_some(wizard.filter(|&i| i < WIZARD.len()), |d, at| d.child(stepper(at, t)))
            .child(body)
            .into_any_element()
    }

    fn render_address(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let ready = !self.address.read(cx).value().trim().is_empty() && !self.connecting;
        let this = cx.entity();
        let token = this.clone();
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Connect to Server".into()))
            .child(intro(
                "Its projects show in your grid beside your own, and open the same way.",
                t,
            ))
            .child(label("Address", t))
            .child(field(&self.address, window, cx, t))
            .child(hint(
                "Members sign in with GitHub. A new server is set up from here, with its setup code.",
                t,
            ))
            .child(self.error_line(t))
            .child(footer(
                Some(
                    link("server-use-token", "Use a token instead", t, move |window, cx| {
                        token.update(cx, |d, cx| d.go(Step::Token, window, cx));
                    })
                    .into_any_element(),
                ),
                vec![
                    self.cancel_button(cx),
                    button("server-continue")
                        .primary()
                        .label(if self.connecting { "Checking…" } else { "Continue" })
                        .disabled(!ready)
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_code(&mut self, name: &str, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let ready = !self.code.read(cx).value().trim().is_empty() && !self.connecting;
        let this = cx.entity();
        let back = this.clone();
        div()
            .flex()
            .flex_col()
            .child(heading_owned(format!("Set Up {name}")))
            .child(intro(
                "This server hasn’t been set up yet. Enter the setup code it printed when it was installed. Run nodus setup-code on it for a new one.",
                t,
            ))
            .child(label("Setup code", t))
            .child(field(&self.code, window, cx, t))
            .child(self.error_line(t))
            .child(footer(
                Some(back_link(back, t)),
                vec![
                    self.cancel_button(cx),
                    button("server-claim")
                        .primary()
                        .label(if self.connecting { "Checking…" } else { "Continue" })
                        .disabled(!ready)
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_name(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let ready = !self.name.read(cx).value().trim().is_empty() && !self.connecting;
        let this = cx.entity();
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Name This Server".into()))
            .child(intro(
                "Everyone who connects sees this name, and so does its GitHub App.",
                t,
            ))
            .child(label("Name", t))
            .child(field(&self.name, window, cx, t))
            .child(self.error_line(t))
            .child(footer(
                None,
                vec![
                    self.cancel_button(cx),
                    button("server-name-save")
                        .primary()
                        .label(if self.connecting { "Saving…" } else { "Continue" })
                        .disabled(!ready)
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_create(&mut self, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let ready = !self.connecting && (!self.for_org || !self.org.read(cx).value().trim().is_empty());
        let this = cx.entity();
        let (personal, org, workflows) = (this.clone(), this.clone(), this.clone());
        let for_org = self.for_org;
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Create the GitHub App".into()))
            .child(intro(
                "Sessions push and open pull requests through a GitHub App of this server’s own. GitHub makes it from the settings Nodus sends.",
                t,
            ))
            .child(label("Owner", t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(choice("server-owner-personal", "My personal account", !for_org, t, move |_, cx| {
                        personal.update(cx, |d, cx| {
                            d.for_org = false;
                            cx.notify();
                        });
                    }))
                    .child(choice("server-owner-org", "An organization", for_org, t, move |window, cx| {
                        org.update(cx, |d, cx| {
                            d.for_org = true;
                            super::focus_all(&d.org, window, cx);
                            cx.notify();
                        });
                    })),
            )
            .when(for_org, |d| {
                d.child(div().mt(px(8.)).pl(px(24.)).child(field(&self.org, window, cx, t)))
            })
            .child(
                div()
                    .id("server-workflows")
                    .mt(px(14.))
                    .flex()
                    .gap(px(10.))
                    .on_click(move |_, _, cx| {
                        workflows.update(cx, |d, cx| {
                            d.workflows = !d.workflows;
                            cx.notify();
                        });
                    })
                    .child(div().pt(px(1.)).child(super::checkbox(self.workflows, false, t)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().font_weight(FontWeight::MEDIUM).child("Let sessions change GitHub Actions workflows"))
                            .child(note(
                                "Off unless you need it: a session could push a workflow that sends a repository’s secrets out.",
                                t,
                            )),
                    ),
            )
            .child(
                callout(
                    "On GitHub’s page, tick Enable Device Flow before you create the App. Nodus signs people in with it.",
                    t,
                )
                .mt(px(14.)),
            )
            .child(self.error_line(t))
            .child(footer(
                None,
                vec![
                    self.cancel_button(cx),
                    button("server-create-app")
                        .primary()
                        .label(if self.connecting { "Opening…" } else { "Create GitHub App" })
                        .disabled(!ready)
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_making(&mut self, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let page = self.app_page.clone();
        let back = cx.entity();
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Create the GitHub App".into()))
            .child(intro(
                "Finish on GitHub’s page in your browser. When GitHub sends you back, this goes on by itself.",
                t,
            ))
            .child(callout(
                "Tick Enable Device Flow on GitHub’s page before you create the App. Nodus signs people in with it.",
                t,
            ))
            .child(waiting("server-app-wait", "Waiting for GitHub…", t, cx))
            .child(self.error_line(t))
            .child(footer(
                Some(
                    link("server-app-back", "Start over", t, move |window, cx| {
                        back.update(cx, |d, cx| {
                            if let Step::Github { session, .. } = &d.step {
                                let session = session.clone();
                                d.go(
                                    Step::Github {
                                        session,
                                        phase: Phase::Create,
                                    },
                                    window,
                                    cx,
                                );
                            }
                        });
                    })
                    .into_any_element(),
                ),
                vec![
                    self.cancel_button(cx),
                    button("server-app-reopen")
                        .label("Open GitHub Again")
                        .disabled(page.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(page) = &page {
                                open_url(page, cx);
                            }
                        })
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_install(&mut self, app: &GithubApp, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let this = cx.entity();
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Install It on Repositories".into()))
            .child(intro_owned(
                format!(
                    "{} is made. Install it on the repositories sessions may work on.",
                    app.name
                ),
                t,
            ))
            .child(callout(
                "Pick only repositories you’d trust every session with, never ones that take outside contributions.",
                t,
            ))
            .child(waiting("server-install-wait", "Waiting for an installation…", t, cx))
            .child(self.error_line(t))
            .child(footer(
                None,
                vec![
                    self.cancel_button(cx),
                    button("server-install")
                        .primary()
                        .label("Install on Repositories")
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_sign_in(&mut self, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let Step::SignIn {
            name,
            setup,
            started,
            ended,
            ..
        } = &self.step
        else {
            return div().into_any_element();
        };
        let wizard = setup.is_some();
        let (title, text) = if wizard {
            (
                "Sign In with GitHub".to_owned(),
                "Sign in with the GitHub account you’ll use here. You’ll be this server’s first member.".to_owned(),
            )
        } else {
            (
                format!("Sign In to {name}"),
                format!("Members of {name} sign in with their GitHub account. Type this code on GitHub’s page."),
            )
        };
        let code = started.as_ref().map(|s| s.user_code.clone());
        let ended = ended.clone();
        let this = cx.entity();
        let (copy, token) = (this.clone(), this.clone());
        let copied = self.copied;
        let code_box = div()
            .mt(px(4.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .id("server-sign-in-code")
                    .flex_1()
                    .h(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_MD)
                    .border(px(0.5))
                    .border_color(t.border)
                    .bg(t.bg_sunken)
                    .font_family(fonts::mono())
                    .text_size(TEXT_2XL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if code.is_some() {
                        t.text_primary
                    } else {
                        t.text_tertiary
                    })
                    .child(code.clone().unwrap_or_else(|| "····-····".into())),
            )
            .child(
                button("server-sign-in-copy")
                    .label(if copied { "Copied" } else { "Copy" })
                    .disabled(code.is_none())
                    .on_click(move |_, _, cx| {
                        copy.update(cx, |d, cx| {
                            if let Step::SignIn {
                                started: Some(started), ..
                            } = &d.step
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(started.user_code.clone()));
                                d.copied = true;
                                cx.notify();
                            }
                        });
                    }),
            );
        let status = match (&ended, &code) {
            (Some(_), _) => None,
            (None, Some(_)) => Some(waiting(
                "server-sign-in-wait",
                "Waiting for you to say yes on GitHub…",
                t,
                cx,
            )),
            (None, None) => Some(waiting("server-sign-in-start", "Asking GitHub for a code…", t, cx)),
        };
        let primary = match &ended {
            Some(_) => button("server-sign-in-retry").primary().label("Try Again"),
            None => button("server-sign-in-open")
                .primary()
                .label("Open GitHub")
                .disabled(code.is_none()),
        }
        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
        .into_any_element();
        div()
            .flex()
            .flex_col()
            .child(heading_owned(title))
            .child(intro_owned(text, t))
            // A try that ended shows why, not its spent code.
            .when(ended.is_none(), |d| d.child(code_box))
            .children(status)
            .when_some(ended, |d, message| d.child(error_text(message, t)))
            .child(self.error_line(t))
            .child(footer(
                (!wizard).then(|| {
                    link("server-use-token", "Use a token instead", t, move |window, cx| {
                        token.update(cx, |d, cx| d.go(Step::Token, window, cx));
                    })
                    .into_any_element()
                }),
                vec![self.cancel_button(cx), primary],
            ))
            .into_any_element()
    }

    fn render_members(&mut self, name: &str, t: Tokens, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let typed = !self.invite.read(cx).value().trim().is_empty();
        let this = cx.entity();
        let (invite, skip) = (this.clone(), this.clone());
        let rows = self
            .members
            .iter()
            .map(|member| member_row(member, self.avatars.get(&avatar_key(member)).cloned().flatten(), t));
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Invite Your Team".into()))
            .child(intro_owned(
                format!("Add people by their GitHub login. Nobody is emailed: they connect to {name} with its address and sign in."),
                t,
            ))
            .child(label("GitHub login", t))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1().child(field(&self.invite, window, cx, t)))
                    .child(
                        button("server-invite")
                            .label(if self.connecting { "Inviting…" } else { "Invite" })
                            .disabled(!typed || self.connecting)
                            .on_click(move |_, _, cx| invite.update(cx, ServersDialog::invite_member)),
                    ),
            )
            .child(self.error_line(t))
            .child(
                div()
                    .id("server-members")
                    .mt(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(footer(
                None,
                vec![
                    button("server-members-done")
                        .primary()
                        .label(if self.members.len() > 1 { "Continue" } else { "Skip for Now" })
                        .disabled(self.connecting)
                        .on_click(move |_, window, cx| {
                            skip.update(cx, |d, cx| {
                                if let Step::Members { server_id, .. } = &d.step {
                                    let server_id = server_id.clone();
                                    d.finish_members(server_id, window, cx);
                                }
                            });
                        })
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    /// UI-333: step 4, Claude accounts.
    fn render_accounts(
        &mut self,
        name: &str,
        t: Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let ready = self.account_typed(cx) && !self.connecting;
        let this = cx.entity();
        let (add, done) = (this.clone(), this.clone());
        let none = self.accounts.is_empty();
        let rows = self.accounts.iter().map(|account| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(32.))
                .px(px(4.))
                .child(icon(Icon::Check, 14., 2.5, t.status_done))
                .child(div().font_weight(FontWeight::MEDIUM).child(account.label.clone()))
                .when_some(account.added_by.as_ref(), |d, by| {
                    d.child(
                        div()
                            .text_color(t.text_tertiary)
                            .child(format!("added by @{}", by.login)),
                    )
                })
        });
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Add Claude Accounts".into()))
            .child(intro_owned(
                format!("Sessions on {name} run on Claude accounts members share. Each session takes the least used one, and moves to another when it hits its limit."),
                t,
            ))
            .child(note(crate::settings::servers::TOKEN_HELP, t))
            .child(div().mt(px(12.)).child(label("Label", t)))
            .child(field(&self.account_label, window, cx, t))
            .child(div().mt(px(10.)).child(label("Token", t)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_1().child(field(&self.account_token, window, cx, t)))
                    .child(
                        button("server-account-add")
                            .label(if self.connecting { "Trying…" } else { "Add" })
                            .disabled(!ready)
                            .on_click(move |_, _, cx| add.update(cx, ServersDialog::add_account)),
                    ),
            )
            .child(self.error_line(t))
            .child(
                div()
                    .id("server-accounts")
                    .mt(px(10.))
                    .flex()
                    .flex_col()
                    .max_h(px(130.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .when(none, |d| {
                d.child(
                    div()
                        .mt(px(8.))
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.status_working)
                        .child("Sessions can’t start on this server until it has a Claude account. You can add one later in Settings → Servers."),
                )
            })
            .child(footer(
                None,
                vec![
                    button("server-accounts-done")
                        .primary()
                        .label(if none { "Skip for Now" } else { "Continue" })
                        .disabled(self.connecting)
                        .on_click(move |_, window, cx| {
                            done.update(cx, |d, cx| {
                                if let Step::Accounts { server_id, .. } = &d.step {
                                    let server_id = server_id.clone();
                                    d.finish_accounts(server_id, window, cx);
                                }
                            });
                        })
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    /// UI-335: wizard step 5, a repository the App reaches as the first
    /// project, or Skip.
    fn render_project(
        &mut self,
        server_id: &str,
        name: &str,
        t: Tokens,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let list = self.repo_list(server_id, t, cx);
        let skip = cx.entity();
        let busy = self.adding.is_some() || self.connecting;
        div()
            .flex()
            .flex_col()
            .child(heading_owned("Add a First Project".into()))
            .child(intro_owned(
                format!("Pick a repository to clone on {name}. Each card you start in it gets a branch of its own."),
                t,
            ))
            .child(field(&self.search, window, cx, t))
            .child(
                div()
                    .id("server-first-project")
                    .mt(px(10.))
                    .flex()
                    .flex_col()
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .when_some(self.add_error.clone(), |d, error| d.child(error_text(error, t)))
            .child(footer(
                None,
                vec![
                    button("server-project-skip")
                        .label("Skip for Now")
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            skip.update(cx, |d, cx| {
                                if let Step::Project { server_id, .. } = &d.step {
                                    let server_id = server_id.clone();
                                    d.finish_project(server_id, window, cx);
                                }
                            });
                        })
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn render_done(&mut self, name: &str, t: Tokens, cx: &mut Context<'_, Self>) -> AnyElement {
        let this = cx.entity();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .size(px(36.))
                    .mb(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(t.status_done)
                    .child(icon(Icon::Check, 20., 2.5, t.text_on_accent)),
            )
            .child(heading_owned(format!("{name} Is Ready")))
            .child(intro(
                "Members, devices, Claude accounts and its GitHub App are in Settings → Servers. Add more projects with Add from GitHub in the projects grid.",
                t,
            ))
            .child(footer(
                None,
                vec![
                    button("server-done")
                        .primary()
                        .label("Choose Projects")
                        .on_click(move |_, window, cx| this.update(cx, |d, cx| d.primary(window, cx)))
                        .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    pub(super) fn cancel_button(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let cancel = cx.entity();
        button("server-connect-cancel")
            .label("Cancel")
            .on_click(move |_, _, cx| cancel.update(cx, Self::close))
            .into_any_element()
    }

    pub(super) fn error_line(&self, t: Tokens) -> AnyElement {
        match &self.error {
            Some(error) => error_text(error.clone(), t).into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// Fetches the avatars it hasn't yet (SRV-045: the core only takes
    /// GitHub's avatar host).
    pub(super) fn fetch_avatars(&mut self, members: &[Member], cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        for member in members {
            let Some(url) = member.avatar_url.clone() else {
                continue;
            };
            if self.avatars.contains_key(&url) {
                continue;
            }
            self.avatars.insert(url.clone(), None);
            let nodus = nodus.clone();
            cx.spawn(async move |this, cx| {
                let file = nodus.servers().avatar(&url).await;
                let _ = this.update(cx, |d, cx| {
                    d.avatars.insert(url, file);
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

/// Whether two addresses are one server's, as typed or as kept.
fn same_address(kept: &str, typed: &str) -> bool {
    let bare = |s: &str| s.trim().trim_end_matches('/').trim_start_matches("http://").to_owned();
    let (kept, typed) = (bare(kept), bare(typed));
    let with_port = |s: String| {
        if s.rsplit_once(':')
            .is_some_and(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        {
            s
        } else {
            format!("{s}:7420")
        }
    };
    kept == typed || with_port(kept) == with_port(typed)
}

/// The key an avatar is kept under.
pub(super) fn avatar_key(member: &Member) -> String {
    member.avatar_url.clone().unwrap_or_default()
}

/// A member: their avatar, name and login.
pub(super) fn member_row(member: &Member, avatar_file: Option<std::path::PathBuf>, t: Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(36.))
        .px(px(4.))
        .child(avatar(&member.login, avatar_file, 24., t))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(member.name.clone().unwrap_or_else(|| member.login.clone())),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .text_color(t.text_secondary)
                        .child(format!("@{}", member.login)),
                ),
        )
}

/// The wizard's steps across the top, the one on show in bold, those done
/// ticked.
fn stepper(at: usize, t: Tokens) -> gpui::Div {
    let steps = WIZARD.iter().enumerate().map(|(i, name)| {
        let done = i < at;
        let current = i == at;
        let dot = div()
            .size(px(16.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .text_size(TEXT_XS)
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| {
                if done {
                    d.bg(t.accent).child(icon(Icon::Check, 10., 3., t.text_on_accent))
                } else if current {
                    d.bg(t.accent).text_color(t.text_on_accent).child((i + 1).to_string())
                } else {
                    d.border(px(1.))
                        .border_color(t.border_strong)
                        .text_color(t.text_tertiary)
                        .child((i + 1).to_string())
                }
            });
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(TEXT_SM)
            .text_color(if current { t.text_primary } else { t.text_tertiary })
            .when(current, |d| d.font_weight(FontWeight::MEDIUM))
            .child(dot)
            .child(*name)
    });
    let mut row = div().flex().items_center().gap(px(8.)).mb(px(16.));
    for (i, step) in steps.enumerate() {
        if i > 0 {
            row = row.child(div().w(px(14.)).h(px(1.)).bg(t.border_strong));
        }
        row = row.child(step);
    }
    row
}

fn intro(text: &'static str, t: Tokens) -> gpui::Div {
    intro_owned(text.to_owned(), t)
}

fn intro_owned(text: String, t: Tokens) -> gpui::Div {
    div().mt(px(6.)).mb(px(16.)).text_color(t.text_secondary).child(text)
}

fn hint(text: &'static str, t: Tokens) -> gpui::Div {
    div()
        .mt(px(6.))
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .text_color(t.text_tertiary)
        .child(text)
}

/// Something to do on GitHub's side, said plainly.
fn callout(text: &'static str, t: Tokens) -> gpui::Div {
    div()
        .flex()
        .gap(px(8.))
        .px(px(12.))
        .py(px(10.))
        .rounded(RADIUS_SM)
        .bg(t.bg_sunken)
        .border(px(0.5))
        .border_color(t.border)
        .child(
            div()
                .pt(px(1.))
                .child(icon(Icon::CircleAlert, 14., 2., t.status_waiting)),
        )
        .child(
            div()
                .flex_1()
                .text_size(TEXT_SM)
                .line_height(px(16.8))
                .text_color(t.text_primary)
                .child(text),
        )
}

/// A spinner and what it waits for.
fn waiting(id: &'static str, text: &'static str, t: Tokens, cx: &gpui::App) -> AnyElement {
    div()
        .mt(px(14.))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(TEXT_SM)
        .text_color(t.text_secondary)
        .child(crate::views::card::spinning(
            icon(Icon::Loader2, 13., 2., t.text_secondary),
            SharedString::from(id),
            cx,
        ))
        .child(text)
        .into_any_element()
}

fn error_text(message: String, t: Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id("server-connect-error")
        .mt(px(12.))
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .text_color(t.status_waiting)
        .child(message)
}

/// The buttons at the bottom right, and a link at the left.
fn footer(left: Option<AnyElement>, buttons: Vec<AnyElement>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .mt(px(20.))
        .child(div().flex().children(left))
        .child(div().flex().gap(px(8.)).children(buttons))
}

/// A link that does something in the dialog.
fn link(
    id: &'static str,
    text: &'static str,
    t: Tokens,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .text_size(TEXT_SM)
        .text_color(t.accent)
        .hover(Styled::underline)
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(text)
}

fn back_link(this: gpui::Entity<ServersDialog>, t: Tokens) -> AnyElement {
    link("server-back", "Back", t, move |window, cx| {
        this.update(cx, |d, cx| d.go(Step::Address, window, cx));
    })
    .into_any_element()
}

/// A radio choice.
fn choice(
    id: &'static str,
    text: &'static str,
    on: bool,
    t: Tokens,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let dot = div()
        .size(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .map(|d| {
            if on {
                d.bg(t.accent)
                    .child(div().size(px(6.)).rounded_full().bg(t.text_on_accent))
            } else {
                d.bg(t.bg_elevated).border(px(1.)).border_color(t.border_strong)
            }
        });
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(dot)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_an_address_with_or_without_its_port() {
        assert!(same_address("http://box:7420", "box"));
        assert!(same_address("http://box:7420", "http://box:7420/"));
        assert!(same_address("http://100.64.0.1:7420", "100.64.0.1"));
        assert!(!same_address("http://box:7421", "box"));
        assert!(!same_address("http://box:7420", "other"));
    }
}
