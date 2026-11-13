//! Settings → Servers → Claude accounts (SRV-105, UI-330…UI-332): the
//! server's pool, each account's usage and state, and Add, Rename and
//! Remove, each answered in the server's words.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window, div, px,
};
use nodus_core::hosted::{AccountState, ClaudeAccount, UsageWindow};

use super::{Confirm, Load, ServerManager, failed_row, note_line, waiting_row};
use crate::components::button::button;
use crate::settings::{group, plain_row, row_text};
use crate::theme::tokens::*;

/// What the Add form and the wizard say about where a token comes from.
pub const TOKEN_HELP: &str = "Run claude setup-token on your own computer, sign in with the Claude account to share, and paste the token it prints. The server tries it once before adding it, and never shows it again.";

impl ServerManager {
    /// The pool, as the server has it now.
    pub(super) fn reload_accounts(&mut self, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let accounts = nodus.servers().accounts(&id).await;
            let _ = this.update(cx, |m, cx| {
                m.accounts = super::loaded(accounts);
                cx.notify();
            });
        })
        .detach();
    }

    /// UI-331: Add, once both fields hold something.
    pub(super) fn add_account(&mut self, cx: &mut Context<'_, Self>) {
        let label = self.account_label.read(cx).value().trim().to_owned();
        let token = self.account_token.read(cx).value().trim().to_owned();
        if label.is_empty() || token.is_empty() || self.checking {
            return;
        }
        self.checking = true;
        self.account_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let added = nodus.servers().add_account(&id, &label, &token).await;
            let _ = this.update(cx, |m, cx| {
                m.checking = false;
                match added {
                    Ok(_) => {
                        m.adding_account = false;
                        m.clear_account_form = true;
                        m.reload(cx);
                    }
                    Err(err) => m.account_error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rename_account(&mut self, cx: &mut Context<'_, Self>) {
        let Some((account_id, field)) = self.renaming.clone() else {
            return;
        };
        let label = field.read(cx).value().trim().to_owned();
        if label.is_empty() {
            return;
        }
        self.account_error = None;
        let nodus = self.state.read(cx).nodus.clone();
        let id = self.server_id.clone();
        cx.spawn(async move |this, cx| {
            let renamed = nodus.servers().rename_account(&id, &account_id, &label).await;
            let _ = this.update(cx, |m, cx| {
                match renamed {
                    Ok(_) => {
                        m.renaming = None;
                        m.reload(cx);
                    }
                    Err(err) => m.account_error = Some(err.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// UI-330: the accounts with their usage, and Add.
    pub(super) fn accounts_group(&self, window: &Window, cx: &mut Context<'_, Self>, t: Tokens) -> gpui::Div {
        let now = crate::parity::now_ms(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        match &self.accounts {
            Load::Loading => rows.push(waiting_row("Loading Claude accounts…", t)),
            Load::Failed(error) => rows.push(failed_row(error, t)),
            Load::Loaded(accounts) if accounts.is_empty() && !self.adding_account => rows.push(waiting_row(
                "No Claude accounts yet, so sessions on this server can’t start.",
                t,
            )),
            Load::Loaded(accounts) => {
                for account in accounts {
                    rows.push(self.account_row(account, now, window, cx, t));
                }
            }
        }
        rows.push(if self.adding_account {
            self.add_form(window, cx, t)
        } else {
            let this = cx.entity();
            plain_row(
                row_text(
                    "Add a Claude account".into(),
                    Some("Sessions take the least used account, and move to another when one hits its limit.".into()),
                    t,
                ),
                Some(
                    button(SharedString::from(format!("server-account-add-{}", self.server_id)))
                        .label("Add…")
                        .on_click(move |_, window, cx| {
                            this.update(cx, |m, cx| {
                                m.adding_account = true;
                                m.account_error = None;
                                m.focus_account_label = true;
                                cx.notify();
                            });
                            window.refresh();
                        })
                        .into_any_element(),
                ),
            )
            .into_any_element()
        });
        let mut d = group(Some("Claude accounts"), rows, false, t);
        if let Some(error) = &self.account_error {
            d = d.child(note_line(error.clone(), true, t));
        }
        d
    }

    /// One account: its label, who added it and its use, its state, its
    /// two usage bars, and Rename and Remove (or the question, or the
    /// rename field, in its place).
    fn account_row(
        &self,
        account: &ClaudeAccount,
        now: f64,
        window: &Window,
        cx: &mut Context<'_, Self>,
        t: Tokens,
    ) -> AnyElement {
        if self.confirm == Some(Confirm::Account(account.id.clone())) {
            let sessions = match account.sessions {
                0 => String::new(),
                1 => " Its session moves to another account.".into(),
                n => format!(" Its {n} sessions move to other accounts."),
            };
            let question = format!(
                "Remove {}?{sessions} Nodus forgets the token, but it works until it expires or its owner revokes it in their Claude account.",
                account.label
            );
            return self.confirm_row(question, "Remove", cx, t);
        }
        if let Some((id, field)) = &self.renaming
            && *id == account.id
        {
            let (cancel, save) = (cx.entity(), cx.entity());
            let typed = !field.read(cx).value().trim().is_empty();
            return plain_row(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(crate::servers::field(field, window, cx, t)),
                Some(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(8.))
                        .child(
                            button("server-account-rename-cancel")
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |m, cx| {
                                        m.renaming = None;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            button("server-account-rename-save")
                                .primary()
                                .label("Save")
                                .disabled(!typed)
                                .on_click(move |_, _, cx| save.update(cx, Self::rename_account)),
                        )
                        .into_any_element(),
                ),
            )
            .into_any_element();
        }
        let mut hint = Vec::new();
        if account.state == AccountState::Limited
            && let Some(at) = account.resets_at
        {
            hint.push(format!("Back {}", crate::terminal::text::format_time(at, now)));
        }
        if let Some(by) = &account.added_by {
            hint.push(format!("Added by @{}", by.login));
        }
        match account.last_used_at {
            Some(at) => hint.push(format!("Last used {}", crate::board::cards::present::time_ago(at, now))),
            None => hint.push("Not used yet".into()),
        }
        match account.sessions {
            0 => {}
            1 => hint.push("1 session".into()),
            n => hint.push(format!("{n} sessions")),
        }
        let (rename, remove) = (cx.entity(), cx.entity());
        let (rename_id, remove_id) = (account.id.clone(), account.id.clone());
        let label = account.label.clone();
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(8.))
            .child(
                button(SharedString::from(format!("server-account-rename-{}", account.id)))
                    .ghost()
                    .label("Rename")
                    .on_click(move |_, window, cx| {
                        let id = rename_id.clone();
                        let label = label.clone();
                        rename.update(cx, |m, cx| {
                            let field = crate::components::text_field::new_input(&label, "Label", false, window, cx);
                            m.renaming = Some((id, field));
                            m.focus_rename = true;
                            cx.notify();
                        });
                    }),
            )
            .child(
                button(SharedString::from(format!("server-account-remove-{}", account.id)))
                    .label("Remove")
                    .on_click(move |_, _, cx| {
                        let id = remove_id.clone();
                        remove.update(cx, |m, cx| {
                            m.confirm = Some(Confirm::Account(id));
                            cx.notify();
                        });
                    }),
            );
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(200.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .truncate()
                    .child(account.label.clone()),
            )
            .child(state_badge(account, t));
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(14.))
            .py(px(11.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        div().flex().flex_col().gap(px(3.)).min_w_0().child(title).child(
                            div()
                                .text_size(TEXT_SM)
                                .line_height(px(16.8))
                                .text_color(t.text_secondary)
                                .child(hint.join(" · ")),
                        ),
                    )
                    .child(buttons),
            )
            .child(usage_bar("5 hours", account.five_hour, now, t))
            .child(usage_bar("7 days", account.seven_day, now, t))
            .into_any_element()
    }

    /// UI-331: a label, the token (hidden as it's typed), where it comes
    /// from, and Cancel / Add.
    fn add_form(&self, window: &Window, cx: &mut Context<'_, Self>, t: Tokens) -> AnyElement {
        let ready = !self.account_label.read(cx).value().trim().is_empty()
            && !self.account_token.read(cx).value().trim().is_empty()
            && !self.checking;
        let (cancel, add) = (cx.entity(), cx.entity());
        let field_label = |text: &'static str| {
            div()
                .mb(px(6.))
                .text_size(TEXT_SM)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(14.))
            .py(px(12.))
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Add a Claude account"),
            )
            .child(
                div()
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_secondary)
                    .child(TOKEN_HELP),
            )
            .child(
                div()
                    .child(field_label("Label"))
                    .child(crate::servers::field(&self.account_label, window, cx, t)),
            )
            .child(
                div()
                    .child(field_label("Token"))
                    .child(crate::servers::field(&self.account_token, window, cx, t)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("server-account-add-cancel")
                            .label("Cancel")
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |m, cx| {
                                    m.adding_account = false;
                                    m.account_error = None;
                                    m.clear_account_form = true;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        button("server-account-add-save")
                            .primary()
                            .label(if self.checking { "Trying the token…" } else { "Add" })
                            .disabled(!ready)
                            .on_click(move |_, _, cx| add.update(cx, Self::add_account)),
                    ),
            )
            .into_any_element()
    }

    /// SRV-040: how many Claude accounts a member added, for removing them.
    pub(super) fn accounts_added_by(&self, github_id: u64) -> usize {
        match &self.accounts {
            Load::Loaded(accounts) => accounts
                .iter()
                .filter(|a| a.added_by.as_ref().is_some_and(|by| by.github_id == github_id))
                .count(),
            _ => 0,
        }
    }

    /// UI-332: the member's removal also takes the accounts they added,
    /// ticked at first.
    pub(super) fn with_accounts_row(
        &self,
        count: usize,
        you: bool,
        cx: &mut Context<'_, Self>,
        t: Tokens,
    ) -> AnyElement {
        let this = cx.entity();
        let ticked = self.with_accounts;
        let who = if you { "you" } else { "they" };
        let text = if count == 1 {
            format!("Also remove the Claude account {who} added. Its sessions move to another.")
        } else {
            format!("Also remove the {count} Claude accounts {who} added. Their sessions move to others.")
        };
        div()
            .id("server-member-accounts")
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .pb(px(10.))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                this.update(cx, |m, cx| {
                    m.with_accounts = !m.with_accounts;
                    cx.notify();
                });
            })
            .child(crate::servers::checkbox(ticked, false, t))
            .child(div().text_size(TEXT_SM).text_color(t.text_secondary).child(text))
            .into_any_element()
    }
}

/// `OK`, `Limited` or `Stopped working` (SRV-103).
fn state_badge(account: &ClaudeAccount, t: Tokens) -> AnyElement {
    let (text, color) = match account.state {
        AccountState::Ok => ("OK".to_owned(), t.text_secondary),
        AccountState::Limited => ("Limited".to_owned(), t.status_working),
        AccountState::Failed => (
            match account.failure.as_deref() {
                Some("account_on_hold") => "Stopped working: on hold".to_owned(),
                Some("billing_error") => "Stopped working: billing".to_owned(),
                Some("authentication_failed") => "Stopped working: signed out".to_owned(),
                _ => "Stopped working".to_owned(),
            },
            t.status_waiting,
        ),
    };
    div()
        .min_w_0()
        .px(px(6.))
        .h(px(17.))
        .flex()
        .items_center()
        .truncate()
        .rounded_full()
        .bg(crate::board::cards::fade(color, 0.12))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// A usage window as a bar: its share used, and when it resets (SRV-102).
fn usage_bar(name: &'static str, window: Option<UsageWindow>, now: f64, t: Tokens) -> AnyElement {
    let live = window.filter(|w| w.resets_at.is_none_or(|at| at > now));
    let used = live.map_or(0.0, |w| w.used_percentage.clamp(0.0, 100.0));
    let words = match (window, live) {
        (None, _) => "Not known yet".to_owned(),
        (Some(_), None) => "0% · reset since".to_owned(),
        (_, Some(w)) => match w.resets_at {
            Some(at) => format!(
                "{}% · resets {}",
                used.round(),
                crate::terminal::text::format_time(at, now)
            ),
            None => format!("{}%", used.round()),
        },
    };
    let fill = if used >= 100.0 {
        t.status_waiting
    } else if used >= 80.0 {
        t.status_working
    } else {
        t.accent
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(TEXT_SM)
        .child(div().w(px(52.)).flex_none().text_color(t.text_secondary).child(name))
        .child(
            div().flex_1().h(px(6.)).rounded_full().bg(t.separator).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(fill)
                    .w(gpui::relative(used as f32 / 100.0)),
            ),
        )
        .child(
            div()
                .w(px(205.))
                .flex_none()
                .truncate()
                .text_color(t.text_tertiary)
                .child(words),
        )
        .into_any_element()
}
