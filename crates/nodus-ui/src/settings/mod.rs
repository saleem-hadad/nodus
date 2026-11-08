//! The Settings dialog (`SettingsDialog.tsx`, `SettingsDialog.css`;
//! UI-116…UI-152): Appearance, Focus, Floating capture, Notifications, Voice
//! prompts and Updates.
//!
//! The capture shortcut state (07), the voice key (05) and the update
//! status (06) come from their tracks' core APIs; until those land their
//! rows render as 0.9.4 renders them before the answer arrives.

pub mod preview;

use std::rc::Rc;

use gpui::component::input::{InputEvent, InputState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Corners, CursorStyle, Entity, FocusHandle, Focusable, FontWeight, ImageSource,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, img, px,
};
use nodus_domain::{
    AppSettings, CaptureShortcut, SettingsPatch, ThemePreference, UpdateState, UpdateStatus, VoiceKeySource,
};

use crate::components::button::button;
use crate::components::focus_ring::focus_ring;
use crate::components::icon::Icon;
use crate::components::modal::Modal;
use crate::components::switch::switch;
use crate::components::text_field::{input, new_input};
use crate::platform::Platform;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// Saves a settings change.
type UpdateSettings = Rc<dyn Fn(SettingsPatch, &mut App)>;

const RELEASES_URL: &str = "https://github.com/saleem-hadad/nodus/releases/latest";

/// The theme tiles, in order.
const THEMES: [(ThemePreference, &str); 3] = [
    (ThemePreference::System, "System"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
];

pub struct SettingsDialog {
    state: Entity<AppState>,
    /// What had focus before, to give it back on close (UI-119).
    pub previous_focus: Option<FocusHandle>,
    panel: FocusHandle,
    stops: Stops,
    key_field: Entity<InputState>,
    /// The capture shortcut's state, once read (07).
    shortcut: Option<CaptureShortcut>,
    /// Where the voice key comes from; None while loading (05).
    key_source: Option<Option<VoiceKeySource>>,
    key_error: Option<String>,
    serial: usize,
}

/// The focusable controls, for Tab (UI-123).
struct Stops {
    close: FocusHandle,
    tile: FocusHandle,
    pin: FocusHandle,
    focus: FocusHandle,
    capture: FocusHandle,
    notify: FocusHandle,
    dock: FocusHandle,
    save: FocusHandle,
    remove: FocusHandle,
    check: FocusHandle,
    auto_update: FocusHandle,
}

impl SettingsDialog {
    pub fn new(
        state: Entity<AppState>,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // UI-118: opening re-reads the settings.
        state.update(cx, AppState::load_settings);
        let panel = cx.focus_handle();
        // UI-119: focus moves into the dialog, onto the panel itself.
        window.focus(&panel, cx);
        let key_field = new_input("", "xAI API key", true, window, cx);
        cx.subscribe_in(&key_field, window, |this, field, e: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let value = field.read(cx).value().to_string();
                this.save_key(&value, cx);
            }
            cx.notify();
        })
        .detach();
        let stops = Stops {
            close: cx.focus_handle(),
            tile: cx.focus_handle(),
            pin: cx.focus_handle(),
            focus: cx.focus_handle(),
            capture: cx.focus_handle(),
            notify: cx.focus_handle(),
            dock: cx.focus_handle(),
            save: cx.focus_handle(),
            remove: cx.focus_handle(),
            check: cx.focus_handle(),
            auto_update: cx.focus_handle(),
        };
        Self {
            state,
            previous_focus,
            panel,
            stops,
            key_field,
            shortcut: None,
            key_source: None,
            key_error: None,
            serial: 0,
        }
    }

    /// UI-142: saves a non-blank key. The voice key API comes with the voice
    /// track (05); until then nothing is saved.
    fn save_key(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        if value.trim().is_empty() {
            return;
        }
        self.key_error = None;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.state.update(cx, AppState::close_settings);
    }

    /// The tab stops now, in document order, counting only the checked
    /// theme radio and enabled controls (UI-123).
    fn tab_stops(
        &self,
        loaded: bool,
        key_source: Option<Option<VoiceKeySource>>,
        update: Option<&UpdateStatus>,
        key_blank: bool,
        cx: &App,
    ) -> Vec<FocusHandle> {
        let mut stops = vec![self.stops.close.clone()];
        if loaded {
            stops.extend([
                self.stops.tile.clone(),
                self.stops.pin.clone(),
                self.stops.focus.clone(),
                self.stops.capture.clone(),
            ]);
            stops.push(self.stops.notify.clone());
            if Platform::CURRENT.is_mac() {
                stops.push(self.stops.dock.clone());
            }
        }
        if key_source.is_some() {
            stops.push(self.key_field.read(cx).focus_handle(cx));
        }
        if !key_blank {
            stops.push(self.stops.save.clone());
        }
        if key_source == Some(Some(VoiceKeySource::Settings)) {
            stops.push(self.stops.remove.clone());
        }
        let state = update.map(|u| &u.state);
        let busy = matches!(state, Some(UpdateState::Checking | UpdateState::Downloading { .. }));
        match state {
            Some(UpdateState::Off { .. }) => {}
            Some(UpdateState::Ready { .. }) => stops.push(self.stops.check.clone()),
            _ if update.is_some() && !busy => stops.push(self.stops.check.clone()),
            _ => {}
        }
        if loaded && !matches!(state, Some(UpdateState::Off { .. })) {
            stops.push(self.stops.auto_update.clone());
        }
        stops
    }

    fn on_key_down(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        stops: &[FocusHandle],
    ) {
        match e.keystroke.key.as_str() {
            // UI-121
            "escape" => {
                cx.stop_propagation();
                self.close(cx);
            }
            // UI-123: Tab cycles inside the dialog.
            "tab" if !stops.is_empty() => {
                cx.stop_propagation();
                let at = stops.iter().position(|f| f.is_focused(window));
                let next = if e.keystroke.modifiers.shift {
                    match at {
                        Some(0) | None => stops.len() - 1,
                        Some(i) => i - 1,
                    }
                } else {
                    match at {
                        Some(i) if i + 1 < stops.len() => i + 1,
                        _ => 0,
                    }
                };
                window.focus(&stops[next], cx);
            }
            _ => {}
        }
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let platform = Platform::CURRENT;
        let (settings, update_status) = {
            let s = self.state.read(cx);
            (s.settings.clone(), s.update_status.clone())
        };
        let loaded = settings.is_some();
        let key_value = self.key_field.read(cx).value().to_string();
        let key_blank = key_value.trim().is_empty();
        let stops = self.tab_stops(loaded, self.key_source, update_status.as_ref(), key_blank, cx);
        let keyboard = window.last_input_was_keyboard();
        let state = self.state.clone();
        let update = move |patch: SettingsPatch, cx: &mut App| state.update(cx, |s, cx| s.update_settings(patch, cx));
        let update: UpdateSettings = Rc::new(update);
        let s = settings.clone().unwrap_or_default();

        // A switch row: clicking anywhere on it toggles (UI-138).
        let switch_row = |id: &'static str,
                          title: &str,
                          hint: Option<String>,
                          checked: bool,
                          focus: &FocusHandle,
                          patch: fn(bool) -> SettingsPatch,
                          update: UpdateSettings| {
            let toggle = update.clone();
            div()
                .id(SharedString::from(format!("{id}-row")))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .when(loaded, |d| d.on_click(move |_, _, cx| toggle(patch(!checked), cx)))
                .child(row_text(title.to_owned(), hint, t))
                .child(
                    switch(id, checked)
                        .disabled(!loaded)
                        .focus(focus)
                        .on_change(move |v, _, cx| update(patch(v), cx)),
                )
        };

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .pt(px(14.))
            .pr(px(12.))
            .pb(px(12.))
            .pl(px(22.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .font_family(fonts::DISPLAY)
                    .text_size(TEXT_LG)
                    .line_height(px(21.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child({
                let this = cx.entity();
                button("settings-close")
                    .ghost()
                    .square()
                    .icon(Icon::X, 16., 2.)
                    .colors(t.text_primary, t.text_primary)
                    .tooltip("Close (Esc)")
                    .focus(&self.stops.close)
                    .on_click(move |_, _, cx| this.update(cx, SettingsDialog::close))
            });

        // Appearance
        let tiles = div().flex().gap(px(16.)).children(THEMES.iter().map(|&(value, label)| {
            let checked = s.theme == value && loaded;
            let hovered = window.use_keyed_state(("theme-tile-hover", value as usize), cx, |_, _| false);
            let hover = *hovered.read(cx);
            let image = preview::preview_image(value, t.status_working, t.status_done, window.scale_factor(), cx);
            let shadows = if checked {
                vec![ring(4., t.accent), ring(2., t.bg_elevated)]
            } else if hover && loaded {
                let mut v = t.md();
                v.push(ring(0.5, t.border_strong));
                v
            } else {
                let mut v = t.sm();
                v.push(ring(0.5, t.border_strong));
                v
            };
            let focused = checked && self.stops.tile.is_focused(window) && keyboard;
            let update = update.clone();
            let tile_focus = self.stops.tile.clone();
            div()
                .id(("theme-tile", value as usize))
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .when(checked, |d| d.track_focus(&tile_focus))
                .when(loaded, |d| {
                    d.on_hover(move |h, _, cx| {
                        hovered.update(cx, |s, cx| {
                            *s = *h;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, _, cx| {
                        update(
                            SettingsPatch {
                                theme: Some(value),
                                ..SettingsPatch::default()
                            },
                            cx,
                        );
                    })
                })
                .child(
                    div()
                        .relative()
                        .w(px(preview::WIDTH))
                        .h(px(preview::HEIGHT))
                        .rounded(px(7.))
                        .shadow(shadows)
                        .when_some(image, |d, image| {
                            d.child(
                                img(ImageSource::Render(image))
                                    .w(px(preview::WIDTH))
                                    .h(px(preview::HEIGHT)),
                            )
                        })
                        .when(focused, |d| {
                            d.child(focus_ring(Corners::all(px(7.)), px(5.), t.focus_ring))
                        }),
                )
                .child(
                    div()
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(if checked { t.text_primary } else { t.text_secondary })
                        .when(checked, |d| d.font_weight(FontWeight::MEDIUM))
                        .child(label),
                )
        }));
        let appearance = group(
            "Appearance",
            vec![
                tiles.into_any_element(),
                hint(
                    format!(
                        "System follows the appearance set in {}.",
                        if platform.is_mac() { "macOS" } else { "Windows" }
                    ),
                    t,
                )
                .into_any_element(),
                switch_row(
                    "pin-rail",
                    "Always show the project sidebar",
                    Some("When off, point at the window’s left edge to slide it in.".into()),
                    s.pin_project_rail && loaded,
                    &self.stops.pin,
                    |v| SettingsPatch {
                        pin_project_rail: Some(v),
                        ..SettingsPatch::default()
                    },
                    update.clone(),
                )
                .mt(px(14.))
                .into_any_element(),
            ],
            true,
            t,
        );

        let focus = group(
            "Focus",
            vec![
                switch_row(
                    "focus-feature",
                    "Focus mode",
                    Some("Adds a Focus switch to the title bar that shows only the projects in focus. Right-click a project and choose Add to Focus.".into()),
                    s.focus_feature && loaded,
                    &self.stops.focus,
                    |v| SettingsPatch {
                        focus_feature: Some(v),
                        ..SettingsPatch::default()
                    },
                    update.clone(),
                )
                .into_any_element(),
            ],
            false,
            t,
        );

        let mut capture_rows: Vec<AnyElement> = vec![
            switch_row(
                "capture",
                "Double-tap ⇧ in any app to capture",
                Some("A panel floats up over what you’re doing. Type or say what you want, and Claude works out whether it’s a new session, a note, a view or a reply to a session.".into()),
                s.capture && loaded,
                &self.stops.capture,
                |v| SettingsPatch {
                    capture: Some(v),
                    ..SettingsPatch::default()
                },
                update.clone(),
            )
            .into_any_element(),
        ];
        // UI-133, UI-134: rows by the shortcut's state (07).
        if self.shortcut == Some(CaptureShortcut::NeedsPermission) {
            capture_rows.push(
                plain_row(
                    row_text(
                        "Allow Nodus in Accessibility".into(),
                        Some("macOS lets an app hear a key pressed in other apps only with this permission. Click Allow… and turn Nodus on in Privacy & Security › Accessibility. Already on there? It was given to another copy of Nodus: Allow… clears it so macOS asks again.".into()),
                        t,
                    ),
                    Some(button("capture-allow").primary().label("Allow…").into_any_element()),
                )
                .mt(px(14.))
                .into_any_element(),
            );
        }
        if self.shortcut == Some(CaptureShortcut::Unavailable) {
            capture_rows.push(
                plain_row(
                    row_hint_only(
                        "The double tap couldn’t start on this computer. File › Capture… still opens the panel.",
                        t,
                    ),
                    None,
                )
                .mt(px(14.))
                .into_any_element(),
            );
        }
        if loaded && s.capture {
            // UI-135
            let ready = if self.shortcut == Some(CaptureShortcut::Ready) {
                "Ready. "
            } else {
                ""
            };
            capture_rows.push(
                plain_row(
                    row_hint_only(&format!("{ready}File › Capture… opens it too."), t),
                    // Opens the capture panel, once the capture track is in (07).
                    Some(button("capture-try").label("Try It").into_any_element()),
                )
                .mt(px(14.))
                .into_any_element(),
            );
        }
        let capture = group("Floating capture", capture_rows, false, t);

        let mut notify_rows = vec![
            switch_row(
                "notify",
                "Notify me when a session needs me or finishes",
                Some("Only while Nodus is in the background. Click one to open its card.".into()),
                s.notify && loaded,
                &self.stops.notify,
                |v| SettingsPatch {
                    notify: Some(v),
                    ..SettingsPatch::default()
                },
                update.clone(),
            )
            .into_any_element(),
        ];
        // UI-137: only a Mac has a Dock.
        if platform.is_mac() {
            notify_rows.push(
                switch_row(
                    "dock-badge",
                    "Count sessions that need me on the Dock icon",
                    None,
                    s.dock_badge && loaded,
                    &self.stops.dock,
                    |v| SettingsPatch {
                        dock_badge: Some(v),
                        ..SettingsPatch::default()
                    },
                    update.clone(),
                )
                .mt(px(14.))
                .into_any_element(),
            );
        }
        let notifications = group("Notifications", notify_rows, false, t);

        // Voice prompts (UI-139…UI-146).
        let source = self.key_source;
        let field_focused = self.key_field.read(cx).focus_handle(cx).is_focused(window);
        let save_view = cx.entity();
        let save_value = key_value.clone();
        let key_form = div()
            .flex()
            .gap(px(8.))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .border(px(0.5))
                    .border_color(if field_focused {
                        transparent_c()
                    } else {
                        t.border_strong
                    })
                    .rounded(RADIUS_SM)
                    .bg(t.bg_elevated)
                    .when(source.is_none(), |d| d.opacity(0.5))
                    .child(input(&self.key_field, px(18.), false))
                    .when(field_focused, |d| {
                        d.child(focus_ring(Corners::all(RADIUS_SM), px(0.), t.focus_ring))
                    }),
            )
            .child(
                button("key-save")
                    .label("Save")
                    .disabled(key_blank)
                    .focus(&self.stops.save)
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |d, cx| d.save_key(&save_value, cx));
                    }),
            );
        let key_hint: AnyElement = match source {
            Some(Some(VoiceKeySource::Settings)) => div()
                .flex()
                .flex_wrap()
                .child(if platform.is_mac() {
                    "Saved in your Keychain.\u{a0}"
                } else {
                    "Saved encrypted for your Windows account.\u{a0}"
                })
                .child(link("key-remove", "Remove", None, t))
                .into_any_element(),
            Some(Some(VoiceKeySource::Env)) => div()
                .child("Using XAI_API_KEY from your shell. A key saved here takes its place.")
                .into_any_element(),
            _ => div()
                .flex()
                .flex_wrap()
                .child("Add a key to speak prompts with the mic button. Get one at\u{a0}")
                .child(link("console", "console.x.ai", Some("https://console.x.ai"), t))
                .child(".")
                .into_any_element(),
        };
        let mut voice_rows = vec![
            key_form.into_any_element(),
            hint_el(key_hint, t).into_any_element(),
            hint("Recordings are sent to xAI to be turned into text.".into(), t)
                .mt(px(4.))
                .into_any_element(),
        ];
        if let Some(error) = &self.key_error {
            voice_rows.push(
                div()
                    .mt(px(8.))
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.status_waiting)
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let voice = group("Voice prompts", voice_rows, false, t);

        // Updates (UI-147…UI-151).
        let status_state = update_status.as_ref().map(|u| &u.state);
        let busy = matches!(
            status_state,
            Some(UpdateState::Checking | UpdateState::Downloading { .. })
        );
        let title = match &update_status {
            Some(u) => format!("Nodus {}", u.current),
            None => "Nodus ".into(),
        };
        let text = update_status.as_ref().and_then(|u| update_text(&u.state));
        let update_button: Option<AnyElement> = match status_state {
            Some(UpdateState::Ready { .. }) => Some(
                button("update-install")
                    .primary()
                    .label("Restart to Update")
                    .focus(&self.stops.check)
                    .into_any_element(),
            ),
            Some(UpdateState::Off { .. }) => None,
            _ => Some(
                button("update-check")
                    .label("Check Now")
                    .disabled(update_status.is_none() || busy)
                    .focus(&self.stops.check)
                    .into_any_element(),
            ),
        };
        let mut update_rows = vec![plain_row(row_text(title, text, t), update_button).into_any_element()];
        if !matches!(status_state, Some(UpdateState::Off { .. })) {
            update_rows.push(
                switch_row(
                    "auto-update",
                    "Download updates automatically",
                    Some(
                        if platform.is_mac() {
                            "They install when you quit Nodus. Sessions keep running."
                        } else {
                            "They install when you choose Restart to Update, which stops the sessions."
                        }
                        .into(),
                    ),
                    s.auto_update && loaded,
                    &self.stops.auto_update,
                    |v| SettingsPatch {
                        auto_update: Some(v),
                        ..SettingsPatch::default()
                    },
                    update.clone(),
                )
                .mt(px(14.))
                .into_any_element(),
            );
        }
        if matches!(status_state, Some(UpdateState::Error { .. })) {
            update_rows.push(
                hint_el(
                    div()
                        .flex()
                        .flex_wrap()
                        .child("You can also\u{a0}")
                        .child(link("releases", "download the latest version", Some(RELEASES_URL), t))
                        .child("\u{a0}yourself.")
                        .into_any_element(),
                    t,
                )
                .into_any_element(),
            );
        }
        let updates = group("Updates", update_rows, false, t);

        let this = cx.entity();
        let close = cx.entity();
        let panel = div()
            .id("settings-inner")
            .track_focus(&self.panel)
            .flex()
            .flex_col()
            .min_h_0()
            .on_key_down(move |e, window, cx| {
                let stops = stops.clone();
                this.update(cx, |d, cx| d.on_key_down(e, window, cx, &stops));
            })
            .child(header)
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(18.))
                    .px(px(22.))
                    .pb(px(22.))
                    .flex()
                    .flex_col()
                    .child(appearance)
                    .child(focus)
                    .child(capture)
                    .child(notifications)
                    .child(voice)
                    .child(updates),
            );
        self.serial = self.serial.max(1);
        Modal::new(("settings", self.serial), px(440.), move |_, cx| {
            close.update(cx, SettingsDialog::close);
        })
        .render(panel, window, cx)
    }
}

fn transparent_c() -> gpui::Rgba {
    crate::components::button::transparent()
}

/// A group: its label, then its rows; 26px above every group but the first.
fn group(label: &str, rows: Vec<AnyElement>, first: bool, t: Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .when(!first, |d| d.mt(px(26.)))
        .child(
            div()
                .mb(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(label.to_owned()),
        )
        .children(rows)
}

/// A row's title and hint.
fn row_text(title: String, hint: Option<String>, t: Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .min_w_0()
        .child(div().font_weight(FontWeight::MEDIUM).child(title))
        .when_some(hint, |d, hint| {
            d.child(
                div()
                    .text_size(TEXT_SM)
                    .line_height(px(16.8))
                    .text_color(t.text_secondary)
                    .child(hint),
            )
        })
}

fn row_hint_only(text: &str, t: Tokens) -> gpui::Div {
    div().flex().flex_col().min_w_0().child(
        div()
            .text_size(TEXT_SM)
            .line_height(px(16.8))
            .text_color(t.text_secondary)
            .child(text.to_owned()),
    )
}

/// A row with its control (a button) at the right; clicks don't toggle.
fn plain_row(text: gpui::Div, control: Option<AnyElement>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(text)
        .children(control)
}

/// A hint paragraph (14px above).
fn hint(text: String, t: Tokens) -> gpui::Div {
    hint_el(div().child(text).into_any_element(), t)
}

fn hint_el(content: AnyElement, t: Tokens) -> gpui::Div {
    div()
        .mt(px(14.))
        .text_size(TEXT_SM)
        .line_height(px(16.8))
        .text_color(t.text_secondary)
        .child(content)
}

/// An inline link (UI-152: opens in the browser).
fn link(id: &'static str, text: &'static str, url: Option<&'static str>, t: Tokens) -> AnyElement {
    div()
        .id(id)
        .text_color(t.accent)
        .cursor(CursorStyle::Arrow)
        .hover(Styled::underline)
        .when_some(url, |d, url| {
            d.on_click(move |_, _, _| {
                nodus_platform::shell::open_external(url);
            })
        })
        .child(text)
        .into_any_element()
}

/// UI-148: the update status, as the Updates row's hint.
pub fn update_text(state: &UpdateState) -> Option<String> {
    match state {
        UpdateState::Off { reason } => Some(reason.clone()),
        UpdateState::Idle => None,
        UpdateState::Checking => Some("Checking for updates…".into()),
        UpdateState::Latest => Some("You’re up to date.".into()),
        UpdateState::Downloading { version, percent } => Some(format!("Downloading version {version}… {percent}%")),
        UpdateState::Ready { version } => Some(format!("Version {version} is ready to install.")),
        UpdateState::Error { message } => Some(message.clone()),
    }
}

#[allow(dead_code)]
fn _settings(_: AppSettings) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_the_update_status() {
        assert_eq!(update_text(&UpdateState::Idle), None);
        assert_eq!(
            update_text(&UpdateState::Checking).as_deref(),
            Some("Checking for updates…")
        );
        assert_eq!(update_text(&UpdateState::Latest).as_deref(), Some("You’re up to date."));
        assert_eq!(
            update_text(&UpdateState::Downloading {
                version: "0.9.5".into(),
                percent: 42
            })
            .as_deref(),
            Some("Downloading version 0.9.5… 42%")
        );
        assert_eq!(
            update_text(&UpdateState::Ready {
                version: "0.9.5".into()
            })
            .as_deref(),
            Some("Version 0.9.5 is ready to install.")
        );
    }
}
