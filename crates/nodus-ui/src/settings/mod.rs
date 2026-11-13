//! The Settings dialog (`SettingsDialog.tsx`, `SettingsDialog.css`;
//! UI-116…UI-152): a sidebar of sections (Appearance, Board, Capture &
//! Voice, Notifications, Updates, Servers) beside the one on show.
//!
//! The voice key (05) and the update status (06) come from the core; the
//! capture shortcut state (07) comes with the capture panel track, and until
//! then its rows render as 0.9.4 renders them before the answer arrives.

pub mod preview;
pub mod servers;

use std::rc::Rc;

use gpui::component::input::{IndentInline, InputEvent, InputState, OutdentInline};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Corners, CursorStyle, Entity, FocusHandle, Focusable, FontWeight, ImageSource,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, actions, canvas, div, img, point, px,
};
use nodus_domain::{
    AppSettings, CaptureShortcut, SettingsPatch, ThemePreference, UpdateState, UpdateStatus, VoiceKeySource,
};

use crate::components::button::button;
use crate::components::focus_ring::focus_ring;
use crate::components::icon::{Icon, icon};
use crate::components::modal::Modal;
use crate::components::scroll::scroll_area;
use crate::components::switch::switch;
use crate::components::text_field::{input, new_input};
use crate::platform::Platform;
use crate::state::AppState;
use crate::theme::tokens::*;
use crate::theme::{self, fonts};

/// Saves a settings change.
type UpdateSettings = Rc<dyn Fn(SettingsPatch, &mut App)>;

actions!(settings, [NextStop, PreviousStop]);

/// The dialog's key context: its Tab and Shift+Tab outrank the window
/// root's, which would walk GPUI's own tab order out of the dialog.
const KEY_CONTEXT: &str = "Settings";

/// Tab and Shift+Tab cycle the dialog's own stops (UI-123).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextStop, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", PreviousStop, Some(KEY_CONTEXT)),
    ]);
}

const RELEASES_URL: &str = "https://github.com/saleem-hadad/nodus/releases/latest";

/// The dialog's size; it's never taller than the window less 48px.
const PANEL_W: f32 = 680.;
const PANEL_H: f32 = 540.;
const SIDEBAR_W: f32 = 188.;
/// The sidebar's title row and the pane's header.
const HEADER_H: f32 = 54.;

/// The sections, down the sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Board,
    CaptureVoice,
    Notifications,
    Updates,
    /// The Nodus servers this app connects to (SRV-165).
    Servers,
}

impl Section {
    pub const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Board,
        Self::CaptureVoice,
        Self::Notifications,
        Self::Updates,
        Self::Servers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Board => "Board",
            Self::CaptureVoice => "Capture & Voice",
            Self::Notifications => "Notifications",
            Self::Updates => "Updates",
            Self::Servers => "Servers",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Appearance => Icon::SunMoon,
            Self::Board => Icon::LayoutDashboard,
            Self::CaptureVoice => Icon::Mic,
            Self::Notifications => Icon::Bell,
            Self::Updates => Icon::Download,
            Self::Servers => Icon::Server,
        }
    }

    /// The section `by` places down the sidebar, stopping at its ends.
    fn step(self, by: isize) -> Self {
        let at = Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as isize;
        Self::ALL[(at + by).clamp(0, Self::ALL.len() as isize - 1) as usize]
    }
}

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
    /// UI-132: reads it while Settings is open.
    _shortcut_poll: gpui::Task<()>,
    /// Where the voice key comes from; None while loading (05).
    key_source: Option<Option<VoiceKeySource>>,
    key_error: Option<String>,
    scroll: ScrollHandle,
    /// The section drawn last, to scroll a newly shown one to its top.
    shown: Option<Section>,
    /// Each server's members, devices, GitHub and activity, by its id.
    managers: Vec<(String, Entity<servers::ServerManager>)>,
    serial: usize,
}

/// The focusable controls, for Tab (UI-123).
struct Stops {
    /// The sidebar's selected section.
    nav: FocusHandle,
    close: FocusHandle,
    tile: FocusHandle,
    pin: FocusHandle,
    wheel: FocusHandle,
    focus: FocusHandle,
    capture: FocusHandle,
    notify: FocusHandle,
    dock: FocusHandle,
    save: FocusHandle,
    remove: FocusHandle,
    check: FocusHandle,
    auto_update: FocusHandle,
    /// Servers: Connect to Server…, then each server's Show Projects… and
    /// Disconnect, by its id.
    connect: FocusHandle,
    servers: Vec<(String, FocusHandle, FocusHandle)>,
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
        cx.subscribe_in(&key_field, window, |this, field, e: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let value = field.read(cx).value().to_string();
                this.save_key(&value, window, cx);
            }
            cx.notify();
        })
        .detach();
        let nodus = state.read(cx).nodus.clone();
        // UI-139: the key field waits for where the key comes from.
        let voice = nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let source = voice.voice().key_source().await;
            let _ = this.update_in(cx, |this, window, cx| this.set_source(source, window, cx));
        })
        .detach();
        // UPD-080: the update status as it is now; changes arrive as events.
        let status_state = state.clone();
        cx.spawn(async move |_, cx| {
            let status = nodus.updates().status().await;
            status_state.update(cx, |s, cx| {
                s.update_status = Some(status);
                cx.notify();
            });
        })
        .detach();
        let stops = Stops {
            nav: cx.focus_handle(),
            close: cx.focus_handle(),
            tile: cx.focus_handle(),
            pin: cx.focus_handle(),
            wheel: cx.focus_handle(),
            focus: cx.focus_handle(),
            capture: cx.focus_handle(),
            notify: cx.focus_handle(),
            dock: cx.focus_handle(),
            save: cx.focus_handle(),
            remove: cx.focus_handle(),
            check: cx.focus_handle(),
            auto_update: cx.focus_handle(),
            connect: cx.focus_handle(),
            servers: Vec::new(),
        };
        let shortcut_poll = poll_shortcut(&state, cx);
        Self {
            state,
            previous_focus,
            panel,
            stops,
            key_field,
            shortcut: None,
            _shortcut_poll: shortcut_poll,
            key_source: None,
            key_error: None,
            scroll: ScrollHandle::new(),
            shown: None,
            managers: Vec::new(),
            serial: 0,
        }
    }

    /// UI-139, UI-140: where the key comes from, and the field's placeholder for it.
    fn set_source(&mut self, source: Option<VoiceKeySource>, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.key_source = Some(source);
        let placeholder = if source == Some(VoiceKeySource::Settings) {
            "Replace the saved xAI API key"
        } else {
            "xAI API key"
        };
        self.key_field
            .update(cx, |field, cx| field.set_placeholder(placeholder, window, cx));
        cx.notify();
    }

    /// UI-142, UI-145: saves a non-blank key (the core trims it), then shows
    /// where the key comes from and empties the field, focus staying in it; a
    /// failure shows its message until the next try.
    fn save_key(&mut self, value: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if value.trim().is_empty() {
            return;
        }
        self.key_error = None;
        cx.notify();
        let nodus = self.state.read(cx).nodus.clone();
        let key = value.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            let saved = nodus.voice().set_key(Some(&key)).await;
            let _ = this.update_in(cx, |this, window, cx| match saved {
                Ok(source) => {
                    this.set_source(source, window, cx);
                    this.key_field.update(cx, |field, cx| field.set_value("", window, cx));
                }
                Err(err) => {
                    this.key_error = Some(err.message);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// UI-144: forgets the saved key; the shell's takes over, if there is one.
    fn remove_key(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let nodus = self.state.read(cx).nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let removed = nodus.voice().set_key(None).await;
            let _ = this.update_in(cx, |this, window, cx| match removed {
                Ok(source) => this.set_source(source, window, cx),
                Err(err) => {
                    this.key_error = Some(err.message);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.state.update(cx, AppState::close_settings);
    }

    /// Shows `section`; the next opening starts there too.
    fn show(&mut self, section: Section, cx: &mut Context<'_, Self>) {
        self.state.update(cx, |s, cx| {
            s.settings_section = section;
            cx.notify();
        });
        cx.notify();
    }

    /// The tab stops now, in document order: the sidebar's section, Close,
    /// then the section's enabled controls, counting only the checked theme
    /// radio (UI-123).
    fn tab_stops(
        &self,
        section: Section,
        loaded: bool,
        key_source: Option<Option<VoiceKeySource>>,
        update: Option<&UpdateStatus>,
        key_blank: bool,
        cx: &App,
    ) -> Vec<FocusHandle> {
        let mut stops = vec![self.stops.nav.clone(), self.stops.close.clone()];
        match section {
            Section::Appearance if loaded => stops.extend([self.stops.tile.clone(), self.stops.pin.clone()]),
            Section::Board if loaded => stops.extend([self.stops.wheel.clone(), self.stops.focus.clone()]),
            Section::CaptureVoice => {
                if loaded {
                    stops.push(self.stops.capture.clone());
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
            }
            Section::Notifications if loaded => {
                stops.push(self.stops.notify.clone());
                if Platform::CURRENT.is_mac() {
                    stops.push(self.stops.dock.clone());
                }
            }
            Section::Updates => {
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
            }
            Section::Servers => {
                for (_, show, disconnect) in &self.stops.servers {
                    stops.extend([show.clone(), disconnect.clone()]);
                }
                stops.push(self.stops.connect.clone());
            }
            _ => {}
        }
        stops
    }

    /// UI-123: Tab cycles inside the dialog, Shift+Tab backwards.
    fn tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<'_, Self>, stops: &[FocusHandle]) {
        if stops.is_empty() {
            return;
        }
        let at = stops.iter().position(|f| f.is_focused(window));
        let next = if back {
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

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        match e.keystroke.key.as_str() {
            // UI-121
            "escape" => {
                cx.stop_propagation();
                self.close(cx);
            }
            // The sidebar's sections, as a list.
            key @ ("up" | "down") if self.stops.nav.is_focused(window) => {
                cx.stop_propagation();
                let section = self.state.read(cx).settings_section;
                self.show(section.step(if key == "up" { -1 } else { 1 }), cx);
            }
            _ => {}
        }
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let (settings, update_status, section) = {
            let s = self.state.read(cx);
            (s.settings.clone(), s.update_status.clone(), s.settings_section)
        };
        // A section shown anew, by the sidebar or by whoever opened it, starts at its top.
        if self.shown != Some(section) {
            self.shown = Some(section);
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        let loaded = settings.is_some();
        self.sync_server_stops(cx);
        if section == Section::Servers {
            self.sync_managers(window, cx);
        }
        let key_value = self.key_field.read(cx).value().to_string();
        let key_blank = key_value.trim().is_empty();
        let stops = self.tab_stops(section, loaded, self.key_source, update_status.as_ref(), key_blank, cx);
        let state = self.state.clone();
        let update = move |patch: SettingsPatch, cx: &mut App| state.update(cx, |s, cx| s.update_settings(patch, cx));
        let update: UpdateSettings = Rc::new(update);
        let s = settings.unwrap_or_default();
        let ctx = Ctx { t, loaded, update };

        let body: Vec<AnyElement> = match section {
            Section::Appearance => self.appearance(&s, &ctx, window, cx),
            Section::Board => self.board(&s, &ctx),
            Section::CaptureVoice => self.capture_voice(&s, &ctx, &key_value, key_blank, window, cx),
            Section::Notifications => self.notifications(&s, &ctx),
            Section::Updates => self.updates(&s, &ctx, update_status.as_ref(), cx),
            Section::Servers => self.servers(&ctx, cx),
        };

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .h(px(HEADER_H))
            .pr(px(12.))
            .pl(px(22.))
            .border_b(px(0.5))
            .border_color(t.separator)
            .child(title(section.label()))
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

        let close = cx.entity();
        let height = px(PANEL_H).min(window.viewport_size().height - px(48.));
        // Tab and Shift+Tab: the dialog's own (bound under its key context),
        // and the key field's, which would indent.
        let tab = |back: bool| {
            let this = cx.entity();
            let stops = stops.clone();
            move |window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                this.update(cx, |d, cx| d.tab(back, window, cx, &stops));
            }
        };
        let (next, previous, indent, outdent) = (tab(false), tab(true), tab(false), tab(true));
        let panel = div()
            .id("settings-inner")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.panel)
            .flex()
            .h(height)
            .min_h_0()
            .on_action(move |_: &NextStop, window, cx| next(window, cx))
            .on_action(move |_: &PreviousStop, window, cx| previous(window, cx))
            .capture_action(move |_: &IndentInline, window, cx| indent(window, cx))
            .capture_action(move |_: &OutdentInline, window, cx| outdent(window, cx))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(self.sidebar(section, update_status.as_ref(), window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(scroll_area(
                        ("settings-body", section as usize),
                        &self.scroll,
                        [20., 22., 24., 22.],
                        div().flex().flex_col().children(body),
                        window,
                        cx,
                    )),
            );
        self.serial = self.serial.max(1);
        Modal::new(("settings", self.serial), px(PANEL_W), move |_, cx| {
            close.update(cx, SettingsDialog::close);
        })
        .render(panel, window, cx)
    }
}

/// What every section's rows draw with.
struct Ctx {
    t: Tokens,
    loaded: bool,
    update: UpdateSettings,
}

impl SettingsDialog {
    /// The sections down the left, the one on show selected; it holds the
    /// sidebar's tab stop, and ↑/↓ move through them (UI-123).
    fn sidebar(
        &self,
        section: Section,
        update: Option<&UpdateStatus>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> gpui::Div {
        let t = theme::tokens(cx);
        let keyboard = window.last_input_was_keyboard();
        // A dot by a section with something waiting there.
        let waiting = |item: Section| match item {
            Section::Updates => matches!(update.map(|u| &u.state), Some(UpdateState::Ready { .. })).then_some(t.accent),
            Section::CaptureVoice => {
                (self.shortcut == Some(CaptureShortcut::NeedsPermission)).then_some(t.status_waiting)
            }
            _ => None,
        };
        let rows = Section::ALL.into_iter().map(|item| {
            let selected = item == section;
            let focused = selected && keyboard && self.stops.nav.is_focused(window);
            let this = cx.entity();
            let nav = self.stops.nav.clone();
            div()
                .id(("settings-nav", item as usize))
                .relative()
                .flex()
                .items_center()
                .gap(px(9.))
                .h(px(30.))
                .px(px(9.))
                .rounded(RADIUS_SM)
                .cursor(CursorStyle::Arrow)
                .when(selected, |d| d.bg(t.bg_selected).track_focus(&self.stops.nav))
                .when(!selected, |d| d.hover(|d| d.bg(t.bg_hover)))
                .on_click(move |_, window, cx| {
                    window.focus(&nav, cx);
                    this.update(cx, |d, cx| d.show(item, cx));
                })
                .child(icon(
                    item.icon(),
                    15.,
                    2.,
                    if selected { t.accent } else { t.text_secondary },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(TEXT_MD)
                        .text_color(t.text_primary)
                        .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                        .child(item.label()),
                )
                .when_some(waiting(item), |d, color| {
                    d.child(div().flex_none().size(px(7.)).rounded_full().bg(color))
                })
                .when(focused, |d| {
                    d.child(focus_ring(Corners::all(RADIUS_SM), px(0.), t.focus_ring))
                })
        });
        div()
            .flex_none()
            .w(px(SIDEBAR_W))
            .flex()
            .flex_col()
            .bg(t.bg_sunken)
            // The panel's clip is square (overflow_hidden ignores its
            // rounding), so the sidebar rounds its own outer corners.
            .rounded_tl(RADIUS_XL)
            .rounded_bl(RADIUS_XL)
            .border_r(px(0.5))
            .border_color(t.separator)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(HEADER_H))
                    .pl(px(20.))
                    .child(title("Settings")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(10.))
                    .pt(px(1.))
                    .children(rows),
            )
    }

    /// Appearance: the theme tiles (UI-125…UI-128) and the sidebar (UI-129).
    fn appearance(
        &self,
        s: &AppSettings,
        ctx: &Ctx,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let t = ctx.t;
        let loaded = ctx.loaded;
        let keyboard = window.last_input_was_keyboard();
        // Three tiles sharing the box's width, which a scrollbar beside the
        // body narrows.
        let row_width = window.use_keyed_state("theme-tiles-width", cx, |_, _| px(420.));
        let tile_w = ((f32::from(*row_width.read(cx)) - 32.0) / 3.0).max(1.0);
        let tile_h = tile_w * 10.0 / 16.0;
        let tiles = div().relative().flex().gap(px(16.)).child(
            canvas(
                move |bounds, _, cx| {
                    let width = bounds.size.width;
                    if *row_width.read(cx) != width {
                        row_width.update(cx, |w, cx| {
                            *w = width;
                            cx.notify();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
        let tiles = tiles.children(THEMES.iter().map(|&(value, label)| {
            let checked = s.theme == value && loaded;
            let hovered = window.use_keyed_state(("theme-tile-hover", value as usize), cx, |_, _| false);
            let hover = *hovered.read(cx);
            let image = preview::preview_image(
                value,
                tile_w,
                t.status_working,
                t.status_done,
                window.scale_factor(),
                cx,
            );
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
            let update = ctx.update.clone();
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
                        .w(px(tile_w))
                        .h(px(tile_h))
                        .rounded(px(7.))
                        .shadow(shadows)
                        .when_some(image, |d, image| {
                            d.child(img(ImageSource::Render(image)).w(px(tile_w)).h(px(tile_h)))
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
        let os = if Platform::CURRENT.is_mac() { "macOS" } else { "Windows" };
        vec![
            group(
                Some("Theme"),
                vec![
                    div()
                        .p(px(16.))
                        .child(tiles)
                        .child(hint(format!("System follows the appearance set in {os}."), t))
                        .into_any_element(),
                ],
                true,
                t,
            )
            .into_any_element(),
            group(
                Some("Sidebar"),
                vec![
                    switch_row(
                        "pin-rail",
                        "Always show the project sidebar",
                        Some("When off, point at the window’s left edge to slide it in.".into()),
                        s.pin_project_rail,
                        &self.stops.pin,
                        |v| SettingsPatch {
                            pin_project_rail: Some(v),
                            ..SettingsPatch::default()
                        },
                        ctx,
                    )
                    .into_any_element(),
                ],
                false,
                t,
            )
            .into_any_element(),
        ]
    }

    /// Board: the mouse wheel (UI-306, BOARD-179) and Focus (UI-130).
    fn board(&self, s: &AppSettings, ctx: &Ctx) -> Vec<AnyElement> {
        let keys = if Platform::CURRENT.is_mac() {
            "⇧ or ⌘"
        } else {
            "Shift or Ctrl"
        };
        vec![
            group(
                None,
                vec![
                    switch_row(
                        "wheel-zoom",
                        "Zoom with the mouse wheel",
                        Some(format!(
                            "Hold {keys} while scrolling to pan instead. A trackpad still pans with two fingers and zooms with a pinch."
                        )),
                        s.wheel_zoom,
                        &self.stops.wheel,
                        |v| SettingsPatch {
                            wheel_zoom: Some(v),
                            ..SettingsPatch::default()
                        },
                        ctx,
                    )
                    .into_any_element(),
                    switch_row(
                        "focus-feature",
                        "Focus mode",
                        Some("Adds a Focus switch to the title bar that shows only the projects in focus. Right-click a project and choose Add to Focus.".into()),
                        s.focus_feature,
                        &self.stops.focus,
                        |v| SettingsPatch {
                            focus_feature: Some(v),
                            ..SettingsPatch::default()
                        },
                        ctx,
                    )
                    .into_any_element(),
                ],
                true,
                ctx.t,
            )
            .into_any_element(),
        ]
    }

    /// Capture & Voice: floating capture (UI-131…UI-135) and the voice key
    /// (UI-139…UI-146).
    fn capture_voice(
        &self,
        s: &AppSettings,
        ctx: &Ctx,
        key_value: &str,
        key_blank: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let t = ctx.t;
        let platform = Platform::CURRENT;
        let mut capture_rows: Vec<AnyElement> = vec![
            switch_row(
                "capture",
                "Double-tap ⇧ in any app to capture",
                Some("A panel floats up over what you’re doing. Type or say what you want, and Claude works out whether it’s a new session, a note, a view or a reply to a session.".into()),
                s.capture,
                &self.stops.capture,
                |v| SettingsPatch {
                    capture: Some(v),
                    ..SettingsPatch::default()
                },
                ctx,
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
                    Some({
                        // UI-133: asks for the permission (07) and shows the state it returns.
                        let this = cx.entity();
                        let nodus = self.state.read(cx).nodus.clone();
                        button("capture-allow")
                            .primary()
                            .label("Allow…")
                            .on_click(move |_, _, cx| {
                                let nodus = nodus.clone();
                                let this = this.clone();
                                cx.spawn(async move |cx| {
                                    let shortcut = nodus.capture().allow().await;
                                    this.update(cx, |s, cx| {
                                        s.shortcut = Some(shortcut);
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })
                            .into_any_element()
                    }),
                )
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
                .into_any_element(),
            );
        }
        if ctx.loaded && s.capture {
            // UI-135
            let ready = if self.shortcut == Some(CaptureShortcut::Ready) {
                "Ready. "
            } else {
                ""
            };
            capture_rows.push(
                plain_row(
                    row_hint_only(&format!("{ready}File › Capture… opens it too."), t),
                    // Opens the capture panel (07, CAP-043).
                    Some({
                        let nodus = self.state.read(cx).nodus.clone();
                        button("capture-try")
                            .label("Try It")
                            .on_click(move |_, _, cx| crate::capture::show(&nodus, cx))
                            .into_any_element()
                    }),
                )
                .into_any_element(),
            );
        }

        // The voice key (UI-139…UI-146).
        let source = self.key_source;
        let field_focused = self.key_field.read(cx).focus_handle(cx).is_focused(window);
        let save_view = cx.entity();
        let save_value = key_value.to_owned();
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
                    .on_click(move |_, window, cx| {
                        save_view.update(cx, |d, cx| d.save_key(&save_value, window, cx));
                    }),
            );
        let remove_view = cx.entity();
        let key_hint: AnyElement = match source {
            Some(Some(VoiceKeySource::Settings)) => div()
                .flex()
                .flex_wrap()
                .child(if platform.is_mac() {
                    "Saved in your Keychain.\u{a0}"
                } else {
                    "Saved encrypted for your Windows account.\u{a0}"
                })
                .child(action_link("key-remove", "Remove", t, move |window, cx| {
                    remove_view.update(cx, |d, cx| d.remove_key(window, cx));
                }))
                .into_any_element(),
            Some(Some(VoiceKeySource::Env)) => div()
                .child("Using XAI_API_KEY from your shell. A key saved here takes its place.")
                .into_any_element(),
            _ => div()
                .flex()
                .flex_wrap()
                .child("Add a key to speak prompts with the mic button. Get one at\u{a0}")
                // The full stop wraps with its link.
                .child(
                    div()
                        .flex()
                        .child(link("console", "console.x.ai", Some("https://console.x.ai"), t))
                        .child("."),
                )
                .into_any_element(),
        };
        let voice = div()
            .p(px(14.))
            .child(key_form)
            .child(hint_el(key_hint, t).mt(px(10.)))
            .child(hint("Recordings are sent to xAI to be turned into text.".into(), t).mt(px(4.)))
            .when_some(self.key_error.clone(), |d, error| {
                d.child(
                    div()
                        .mt(px(8.))
                        .text_size(TEXT_SM)
                        .line_height(px(16.8))
                        .text_color(t.status_waiting)
                        .child(error),
                )
            });

        vec![
            group(Some("Floating capture"), capture_rows, true, t).into_any_element(),
            group(Some("Voice prompts"), vec![voice.into_any_element()], false, t).into_any_element(),
        ]
    }

    /// Notifications (UI-136, UI-137).
    fn notifications(&self, s: &AppSettings, ctx: &Ctx) -> Vec<AnyElement> {
        let mut rows = vec![
            switch_row(
                "notify",
                "Notify me when a session needs me or finishes",
                Some("Only while Nodus is in the background. Click one to open its card.".into()),
                s.notify,
                &self.stops.notify,
                |v| SettingsPatch {
                    notify: Some(v),
                    ..SettingsPatch::default()
                },
                ctx,
            )
            .into_any_element(),
        ];
        // UI-137: only a Mac has a Dock.
        if Platform::CURRENT.is_mac() {
            rows.push(
                switch_row(
                    "dock-badge",
                    "Count sessions that need me on the Dock icon",
                    None,
                    s.dock_badge,
                    &self.stops.dock,
                    |v| SettingsPatch {
                        dock_badge: Some(v),
                        ..SettingsPatch::default()
                    },
                    ctx,
                )
                .into_any_element(),
            );
        }
        vec![group(None, rows, true, ctx.t).into_any_element()]
    }

    /// Updates (UI-147…UI-151).
    fn updates(
        &self,
        s: &AppSettings,
        ctx: &Ctx,
        update_status: Option<&UpdateStatus>,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let t = ctx.t;
        let nodus = self.state.read(cx).nodus.clone();
        let status_state = update_status.map(|u| &u.state);
        let busy = matches!(
            status_state,
            Some(UpdateState::Checking | UpdateState::Downloading { .. })
        );
        let title = match update_status {
            Some(u) => format!("Nodus {}", u.current),
            None => "Nodus ".into(),
        };
        let text = update_status.and_then(|u| update_text(&u.state));
        let update_button: Option<AnyElement> = match status_state {
            Some(UpdateState::Ready { .. }) => Some(
                button("update-install")
                    .primary()
                    .label("Restart to Update")
                    .focus(&self.stops.check)
                    .on_click({
                        let nodus = nodus.clone();
                        move |_, _, cx| crate::shell::core_shell::install_update(nodus.clone(), cx)
                    })
                    .into_any_element(),
            ),
            Some(UpdateState::Off { .. }) => None,
            _ => Some(
                button("update-check")
                    .label("Check Now")
                    .disabled(update_status.is_none() || busy)
                    .focus(&self.stops.check)
                    .on_click({
                        let nodus = nodus.clone();
                        move |_, _, cx| {
                            let nodus = nodus.clone();
                            cx.spawn(async move |_| nodus.updates().check().await).detach();
                        }
                    })
                    .into_any_element(),
            ),
        };
        let mut rows = vec![plain_row(row_text(title, text, t), update_button).into_any_element()];
        if !matches!(status_state, Some(UpdateState::Off { .. })) {
            rows.push(
                switch_row(
                    "auto-update",
                    "Download updates automatically",
                    Some(
                        if Platform::CURRENT.is_mac() {
                            "They install when you quit Nodus. Sessions keep running."
                        } else {
                            "They install when you choose Restart to Update, which stops the sessions."
                        }
                        .into(),
                    ),
                    s.auto_update,
                    &self.stops.auto_update,
                    |v| SettingsPatch {
                        auto_update: Some(v),
                        ..SettingsPatch::default()
                    },
                    ctx,
                )
                .into_any_element(),
            );
        }
        let mut body = vec![group(None, rows, true, t).into_any_element()];
        if matches!(status_state, Some(UpdateState::Error { .. })) {
            body.push(
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
                .mt(px(10.))
                .px(px(2.))
                .into_any_element(),
            );
        }
        body
    }
}

impl SettingsDialog {
    /// A Show Projects… and a Disconnect stop for each server, in order.
    fn sync_server_stops(&mut self, cx: &mut Context<'_, Self>) {
        let ids: Vec<String> = self.state.read(cx).servers.iter().map(|s| s.id.clone()).collect();
        let mut stops = std::mem::take(&mut self.stops.servers);
        self.stops.servers = ids
            .into_iter()
            .map(|id| match stops.iter().position(|(known, _, _)| *known == id) {
                Some(at) => stops.swap_remove(at),
                None => (id, cx.focus_handle(), cx.focus_handle()),
            })
            .collect();
    }

    /// A manager for each server, made when it's first shown (UI-323).
    fn sync_managers(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let ids: Vec<String> = self.state.read(cx).servers.iter().map(|s| s.id.clone()).collect();
        let mut managers = std::mem::take(&mut self.managers);
        self.managers = ids
            .into_iter()
            .map(|id| match managers.iter().position(|(known, _)| *known == id) {
                Some(at) => managers.swap_remove(at),
                None => {
                    let state = self.state.clone();
                    let manager =
                        gpui::AppContext::new(cx, |cx| servers::ServerManager::new(state, id.clone(), window, cx));
                    (id, manager)
                }
            })
            .collect();
    }

    /// Servers (SRV-165): each server's address, version, status and who
    /// signed in, Show Projects… and Disconnect; then its members, devices,
    /// GitHub App and recent activity (UI-323…UI-327); last, Connect to
    /// Server…. Claude accounts and updating the server come later.
    fn servers(&self, ctx: &Ctx, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        use crate::state::ServerDialog;
        let t = ctx.t;
        let servers = self.state.read(cx).servers.clone();
        let mut body = Vec::new();
        for (i, server) in servers.iter().enumerate() {
            let Some((_, show_stop, disconnect_stop)) = self.stops.servers.iter().find(|(id, _, _)| *id == server.id)
            else {
                continue;
            };
            let status = crate::servers::status_text(server);
            let mut detail = match &server.version {
                Some(version) => format!("Nodus {version} · {status}"),
                None => status,
            };
            if let Some(login) = &server.login {
                detail.push_str(&format!(" · Signed in as @{login}"));
            }
            let (show, disconnect) = (self.state.clone(), self.state.clone());
            let (show_id, disconnect_id) = (server.id.clone(), server.id.clone());
            let rows = vec![
                plain_row(
                    row_text(server.url.trim_start_matches("http://").to_owned(), Some(detail), t),
                    Some(
                        button(SharedString::from(format!("server-show-{}", server.id)))
                            .label("Show Projects…")
                            .focus(show_stop)
                            .on_click(move |_, _, cx| {
                                let server_id = Some(show_id.clone());
                                show.update(cx, |s, cx| {
                                    s.open_server_dialog(ServerDialog::Projects { server_id }, cx);
                                });
                            })
                            .into_any_element(),
                    ),
                )
                .into_any_element(),
                plain_row(
                    row_text(
                        "Disconnect".into(),
                        Some(
                            "Forgets this server and its projects on this device. Nothing changes on the server."
                                .into(),
                        ),
                        t,
                    ),
                    Some(
                        button(SharedString::from(format!("server-disconnect-{}", server.id)))
                            .kind(crate::components::button::Kind::Danger)
                            .label("Disconnect")
                            .focus(disconnect_stop)
                            .on_click(move |_, _, cx| {
                                let id = disconnect_id.clone();
                                disconnect.update(cx, |s, cx| s.disconnect_server(id, cx));
                            })
                            .into_any_element(),
                    ),
                )
                .into_any_element(),
            ];
            body.push(group(Some(&server.name), rows, i == 0, t).into_any_element());
            if let Some((_, manager)) = self.managers.iter().find(|(id, _)| *id == server.id) {
                body.push(manager.clone().into_any_element());
            }
        }
        let connect = self.state.clone();
        let (title, hint) = if servers.is_empty() {
            (
                "No servers yet",
                "Connect to a Nodus server to show its projects beside your own.",
            )
        } else {
            (
                "Another server",
                "Its projects show beside these servers’ and your own.",
            )
        };
        body.push(
            group(
                None,
                vec![
                    plain_row(
                        row_text(title.into(), Some(hint.into()), t),
                        Some(
                            button("server-connect-open")
                                .label("Connect to Server…")
                                .focus(&self.stops.connect)
                                .on_click(move |_, _, cx| {
                                    connect.update(cx, |s, cx| s.open_server_dialog(ServerDialog::Connect, cx));
                                })
                                .into_any_element(),
                        ),
                    )
                    .into_any_element(),
                ],
                servers.is_empty(),
                t,
            )
            .into_any_element(),
        );
        body
    }
}

fn transparent_c() -> gpui::Rgba {
    crate::components::button::transparent()
}

/// The sidebar's and the pane's titles.
fn title(text: &'static str) -> gpui::Div {
    div()
        .font_family(fonts::DISPLAY)
        .text_size(TEXT_LG)
        .line_height(px(21.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// A group: its label, if it has one, over its rows in a box, a hairline
/// between rows; 24px above every group but the section's first.
fn group(label: Option<&str>, rows: Vec<AnyElement>, first: bool, t: Tokens) -> gpui::Div {
    let rows = rows.into_iter().enumerate().flat_map(|(i, row)| {
        let line = (i > 0).then(|| div().h(px(0.5)).mx(px(14.)).bg(t.separator).into_any_element());
        line.into_iter().chain([row])
    });
    div()
        .flex()
        .flex_col()
        .when(!first, |d| d.mt(px(24.)))
        .when_some(label, |d, label| {
            d.child(
                div()
                    .mb(px(8.))
                    .px(px(2.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text_primary)
                    .child(label.to_owned()),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(RADIUS_MD)
                .border(px(0.5))
                .border_color(t.border)
                .bg(t.bg_sunken)
                .children(rows),
        )
}

/// A switch row: clicking anywhere on it toggles (UI-138); disabled until
/// the settings are loaded (UI-124).
#[allow(clippy::too_many_arguments)]
fn switch_row(
    id: &'static str,
    title: &str,
    hint: Option<String>,
    checked: bool,
    focus: &FocusHandle,
    patch: fn(bool) -> SettingsPatch,
    ctx: &Ctx,
) -> gpui::Stateful<gpui::Div> {
    let loaded = ctx.loaded;
    let checked = checked && loaded;
    let toggle = ctx.update.clone();
    let update = ctx.update.clone();
    div()
        .id(SharedString::from(format!("{id}-row")))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .px(px(14.))
        .py(px(11.))
        .when(loaded, |d| d.on_click(move |_, _, cx| toggle(patch(!checked), cx)))
        .child(row_text(title.to_owned(), hint, ctx.t))
        .child(
            switch(id, checked)
                .disabled(!loaded)
                .focus(focus)
                .on_change(move |v, _, cx| update(patch(v), cx)),
        )
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
        .px(px(14.))
        .py(px(11.))
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

/// A link that does something in the app rather than open a page.
fn action_link(
    id: &'static str,
    text: &'static str,
    t: Tokens,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .text_color(t.accent)
        .cursor(CursorStyle::Arrow)
        .hover(Styled::underline)
        .on_click(move |_, window, cx| on_click(window, cx))
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

/// UI-132: while Settings is open, the capture shortcut's state is read at
/// once and then every 2 s, and again at once when `settings.capture` changes.
fn poll_shortcut(state: &Entity<AppState>, cx: &mut Context<'_, SettingsDialog>) -> gpui::Task<()> {
    const EVERY: std::time::Duration = std::time::Duration::from_millis(2000);
    const STEP: std::time::Duration = std::time::Duration::from_millis(100);
    let state = state.clone();
    cx.spawn(async move |this, cx| {
        loop {
            let (nodus, capture) = cx.update(|cx| {
                let s = state.read(cx);
                (s.nodus.clone(), s.settings.as_ref().map(|s| s.capture))
            });
            let shortcut = nodus.capture().shortcut().await;
            if this
                .update(cx, |s, cx| {
                    if s.shortcut != Some(shortcut) {
                        s.shortcut = Some(shortcut);
                        cx.notify();
                    }
                })
                .is_err()
            {
                return;
            }
            let mut waited = std::time::Duration::ZERO;
            while waited < EVERY {
                cx.background_executor().timer(STEP).await;
                waited += STEP;
                let now = cx.update(|cx| state.read(cx).settings.as_ref().map(|s| s.capture));
                if now != capture {
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_through_the_sections_stopping_at_the_ends() {
        assert_eq!(Section::default(), Section::Appearance);
        assert_eq!(Section::Appearance.step(-1), Section::Appearance);
        assert_eq!(Section::Appearance.step(1), Section::Board);
        assert_eq!(Section::Board.step(1), Section::CaptureVoice);
        assert_eq!(Section::Updates.step(-1), Section::Notifications);
        assert_eq!(Section::Updates.step(1), Section::Servers);
        assert_eq!(Section::Servers.step(1), Section::Servers);
    }

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
