//! The application menu (`src/main/menu.ts`; spec 01 PLAT-037…PLAT-062).
//!
//! On a Mac it's the menu bar; elsewhere the same menu pops up from the
//! title bar's ☰ button (UI-099). Menu shortcuts become menu commands sent to
//! the window (`Event::Menu`), so they work while a terminal has focus.
//!
//! By the user's decision, Force Reload and Toggle Developer Tools are gone,
//! and Reload is there only in debug builds.

use gpui::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, actions};
use nodus_core::Nodus;
use nodus_domain::{AppSettings, Event, MenuCommand};

use crate::platform::Platform;

actions!(
    nodus,
    [
        About,
        CheckForUpdates,
        OpenSettings,
        HideApp,
        HideOthers,
        ShowAll,
        Quit,
        QuitAndStopSessions,
        Capture,
        NewSession,
        NewTerminal,
        NewProject,
        Close,
        SearchSessions,
        ToggleFocusMode,
        Reload,
        ActualSize,
        ZoomIn,
        ZoomOut,
        ToggleFullScreen,
        Minimize,
        Zoom,
        BringAllToFront,
        StarOnGitHub,
        ReportProblem,
        PasteAndMatchStyle,
        DeleteSelection,
    ]
);

pub const REPO_URL: &str = "https://github.com/saleem-hadad/nodus";

/// The menu bar for these settings: View › Focus Mode shows only while focus
/// is offered, ticked while it's on (PLAT-052, PLAT-053).
pub fn app_menus(settings: &AppSettings, platform: Platform) -> Vec<Menu> {
    use gpui::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    let mac = platform.is_mac();
    let check_for_updates = MenuItem::action("Check for Updates…", CheckForUpdates);
    let settings_item = MenuItem::action("Settings…", OpenSettings);
    let quit_and_stop = MenuItem::action("Quit and Stop Sessions", QuitAndStopSessions);

    let mut file = vec![
        MenuItem::action("Capture…", Capture),
        MenuItem::separator(),
        MenuItem::action("New Session", NewSession),
        MenuItem::action("New Terminal", NewTerminal),
        MenuItem::action("New Project…", NewProject),
        MenuItem::separator(),
        MenuItem::action("Close", Close),
    ];
    let mut menus = Vec::new();
    if mac {
        menus.push(Menu::new("Nodus").items([
            MenuItem::action("About Nodus", About),
            check_for_updates,
            MenuItem::separator(),
            settings_item,
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Nodus", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Nodus", Quit),
            quit_and_stop,
        ]));
    } else {
        file.extend([
            MenuItem::separator(),
            settings_item,
            check_for_updates,
            MenuItem::separator(),
            MenuItem::action("Exit", Quit),
            quit_and_stop,
        ]);
    }
    menus.push(Menu::new("File").items(file));

    let mut edit = vec![
        MenuItem::os_action("Undo", Undo, OsAction::Undo),
        MenuItem::os_action("Redo", Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, OsAction::Cut),
        MenuItem::os_action("Copy", Copy, OsAction::Copy),
        MenuItem::os_action("Paste", Paste, OsAction::Paste),
    ];
    if mac {
        edit.push(MenuItem::action("Paste and Match Style", PasteAndMatchStyle));
    }
    edit.extend([
        MenuItem::action("Delete", DeleteSelection),
        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        MenuItem::separator(),
        MenuItem::action("Search Sessions…", SearchSessions),
    ]);
    menus.push(Menu::new("Edit").items(edit));

    let mut view = Vec::new();
    if settings.focus_feature {
        view.push(MenuItem::action("Focus Mode", ToggleFocusMode).checked(settings.focus_mode));
        view.push(MenuItem::separator());
    }
    if cfg!(debug_assertions) {
        view.push(MenuItem::action("Reload", Reload));
        view.push(MenuItem::separator());
    }
    view.extend([
        MenuItem::action("Actual Size", ActualSize),
        MenuItem::action("Zoom In", ZoomIn),
        MenuItem::action("Zoom Out", ZoomOut),
        MenuItem::separator(),
        MenuItem::action("Toggle Full Screen", ToggleFullScreen),
    ]);
    menus.push(Menu::new("View").items(view));

    menus.push(if mac {
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ])
    } else {
        // Its Close would take Ctrl+W, which would close the window.
        Menu::new("Window").items([MenuItem::action("Minimize", Minimize)])
    });

    let mut help = vec![
        MenuItem::action("Star on GitHub", StarOnGitHub),
        MenuItem::action("Report a Problem…", ReportProblem),
    ];
    if !mac {
        help.extend([MenuItem::separator(), MenuItem::action("About Nodus", About)]);
    }
    menus.push(Menu::new("Help").items(help));
    menus
}

/// The menu's shortcuts (PLAT-055): every listed one on a Mac but ⌘K;
/// off a Mac only Ctrl+Shift+N, Ctrl+, and Ctrl+Shift+F (while offered), so
/// Ctrl and a letter stay a focused terminal's.
pub fn key_bindings(settings: &AppSettings, platform: Platform) -> Vec<KeyBinding> {
    if platform.is_mac() {
        let mut bindings = vec![
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-h", HideApp, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("alt-cmd-q", QuitAndStopSessions, None),
            KeyBinding::new("cmd-n", NewSession, None),
            KeyBinding::new("cmd-t", NewTerminal, None),
            KeyBinding::new("shift-cmd-n", NewProject, None),
            KeyBinding::new("cmd-w", Close, None),
            KeyBinding::new("alt-shift-cmd-v", PasteAndMatchStyle, None),
            KeyBinding::new("cmd-0", ActualSize, None),
            KeyBinding::new("cmd-=", ZoomIn, None),
            KeyBinding::new("cmd--", ZoomOut, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ];
        if cfg!(debug_assertions) {
            bindings.push(KeyBinding::new("cmd-r", Reload, None));
        }
        if settings.focus_feature {
            bindings.push(KeyBinding::new("shift-cmd-f", ToggleFocusMode, None));
        }
        bindings
    } else {
        let mut bindings = vec![
            KeyBinding::new("ctrl-shift-n", NewProject, None),
            KeyBinding::new("ctrl-,", OpenSettings, None),
            KeyBinding::new("f11", ToggleFullScreen, None),
        ];
        if settings.focus_feature {
            bindings.push(KeyBinding::new("ctrl-shift-f", ToggleFocusMode, None));
        }
        bindings
    }
}

/// Installs the menu for `settings`, again whenever they change
/// (`syncFocusMenu`).
pub fn install_menus(settings: &AppSettings, cx: &mut App) {
    cx.set_menus(app_menus(settings, Platform::CURRENT));
    #[cfg(target_os = "macos")]
    crate::mac::register_help_menu();
}

/// Binds the menu's shortcuts once. ⇧⌘F stays bound while focus isn't
/// offered; toggling then does nothing (UI-047), as the hidden item did.
pub fn bind_shortcuts(cx: &mut App) {
    let offered = AppSettings {
        focus_feature: true,
        ..AppSettings::default()
    };
    cx.bind_keys(key_bindings(&offered, Platform::CURRENT));
}

/// What each menu item does (PLAT-046…PLAT-059).
pub fn register_actions(nodus: &Nodus, cx: &mut App) {
    let send = |command: MenuCommand| {
        let nodus = nodus.clone();
        move |cx: &mut App| {
            // PLAT-036: with no main window, menu commands go nowhere.
            if crate::shell::window::main_window(cx).is_some() {
                nodus.emit(Event::Menu(command));
            }
        }
    };
    let s = send(MenuCommand::Settings);
    cx.on_action(move |_: &OpenSettings, cx| s(cx));
    let n = nodus.clone();
    cx.on_action(move |_: &CheckForUpdates, cx| crate::shell::core_shell::check_for_updates(n.clone(), cx));
    let s = send(MenuCommand::NewSession);
    cx.on_action(move |_: &NewSession, cx| s(cx));
    let s = send(MenuCommand::NewTerminal);
    cx.on_action(move |_: &NewTerminal, cx| s(cx));
    let s = send(MenuCommand::NewProject);
    cx.on_action(move |_: &NewProject, cx| s(cx));
    let s = send(MenuCommand::Close);
    cx.on_action(move |_: &Close, cx| s(cx));
    let s = send(MenuCommand::Search);
    cx.on_action(move |_: &SearchSessions, cx| s(cx));
    let s = send(MenuCommand::ToggleFocus);
    cx.on_action(move |_: &ToggleFocusMode, cx| s(cx));

    cx.on_action(|_: &About, _| {
        #[cfg(target_os = "macos")]
        crate::mac::show_about_panel();
    });
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| cx.quit());
    let n = nodus.clone();
    cx.on_action(move |_: &QuitAndStopSessions, cx| {
        crate::shell::core_shell::quit_and_stop_sessions(n.clone(), cx);
    });
    // PLAT-048, CAP-042: opens the floating capture panel, with or without a
    // main window, whatever the shortcut's state.
    let capture = nodus.clone();
    cx.on_action(move |_: &Capture, cx| crate::capture::show(&capture, cx));
    cx.on_action(|_: &StarOnGitHub, _| {
        nodus_platform::shell::open_external(REPO_URL);
    });
    // RPT-001: the report dialog, over the main window.
    let s = send(MenuCommand::ReportProblem);
    cx.on_action(move |_: &ReportProblem, cx| s(cx));
    cx.on_action(|_: &ToggleFullScreen, cx| with_main(cx, |w| w.toggle_fullscreen()));
    cx.on_action(|_: &Minimize, cx| with_main(cx, |w| w.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_main(cx, |w| w.zoom_window()));
    cx.on_action(|_: &BringAllToFront, cx| cx.activate(true));
    cx.on_action(|_: &Reload, cx| crate::shell::window::reload(cx));
    // PLAT-057: page zoom scales the whole UI (plan: Decisions for the user).
    cx.on_action(|_: &ActualSize, cx| crate::shell::window::zoom(None, cx));
    cx.on_action(|_: &ZoomIn, cx| crate::shell::window::zoom(Some(nodus_core::zoom::STEP), cx));
    cx.on_action(|_: &ZoomOut, cx| crate::shell::window::zoom(Some(-nodus_core::zoom::STEP), cx));
    cx.on_action(|_: &PasteAndMatchStyle, cx| {
        cx.dispatch_action(&gpui::component::input::Paste);
    });
    // Edit › Delete is Chromium's `delete` role: it removes the selection, and
    // with only a caret it does nothing (Blink's menu Delete needs a range).
    track_text_fields(cx);
    cx.on_action(|_: &DeleteSelection, cx| {
        // The window in front; the main one when none is (the test driver's).
        let Some(window) = cx
            .active_window()
            .or_else(|| crate::shell::window::main_window(cx).map(Into::into))
        else {
            return;
        };
        // Outside the update that dispatched this (the window is in use).
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                if focused_field_has_selection(window, cx) {
                    window.dispatch_action(Box::new(gpui::component::input::Delete), cx);
                }
            });
        });
    });
}

/// Every text field there is, so Edit › Delete can ask the focused one
/// whether anything is selected.
#[derive(Default)]
struct TextFields {
    inputs: Vec<gpui::WeakEntity<gpui::component::input::InputState>>,
    textareas: Vec<gpui::WeakEntity<gpui::component::input::TextareaState>>,
}

impl gpui::Global for TextFields {}

fn track_text_fields(cx: &mut App) {
    cx.observe_new::<gpui::component::input::InputState>(|_, _, cx| {
        let field = cx.entity().downgrade();
        let fields = cx.default_global::<TextFields>();
        fields.inputs.retain(|f| f.upgrade().is_some());
        fields.inputs.push(field);
    })
    .detach();
    cx.observe_new::<gpui::component::input::TextareaState>(|_, _, cx| {
        let field = cx.entity().downgrade();
        let fields = cx.default_global::<TextFields>();
        fields.textareas.retain(|f| f.upgrade().is_some());
        fields.textareas.push(field);
    })
    .detach();
}

/// Whether the field that has the keyboard in `window` has a selection that
/// isn't empty.
fn focused_field_has_selection(window: &gpui::Window, cx: &App) -> bool {
    use gpui::Focusable as _;
    let Some(fields) = cx.try_global::<TextFields>() else {
        return false;
    };
    fields.inputs.iter().filter_map(gpui::WeakEntity::upgrade).any(|f| {
        let f = f.read(cx);
        f.focus_handle(cx).is_focused(window) && !f.selected_range().is_empty()
    }) || fields.textareas.iter().filter_map(gpui::WeakEntity::upgrade).any(|f| {
        let f = f.read(cx);
        f.focus_handle(cx).is_focused(window) && !f.selected_range().is_empty()
    })
}

fn with_main(cx: &mut App, f: impl FnOnce(&mut gpui::Window)) {
    if let Some(handle) = crate::shell::window::main_window(cx) {
        let _ = handle.update(cx, |_, window, _| f(window));
    }
}

/// `process.getSystemVersion()`: `major.minor.patch` on a Mac.
pub(crate) fn system_version() -> String {
    #[cfg(target_os = "macos")]
    {
        crate::mac::os_version()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}

/// Node's name for the CPU: `arm64` or `x64`.
pub(crate) fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_focus_mode_only_while_offered() {
        let names = |settings: &AppSettings| -> Vec<String> {
            app_menus(settings, Platform::Mac)
                .into_iter()
                .find(|m| m.name == "View")
                .unwrap()
                .items
                .iter()
                .filter_map(|i| match i {
                    MenuItem::Action { name, checked, .. } => {
                        Some(format!("{name}{}", if *checked { "✓" } else { "" }))
                    }
                    _ => None,
                })
                .collect()
        };
        let off = AppSettings::default();
        assert!(!names(&off).iter().any(|n| n.starts_with("Focus Mode")));
        let on = AppSettings {
            focus_feature: true,
            focus_mode: true,
            ..AppSettings::default()
        };
        assert_eq!(names(&on)[0], "Focus Mode✓");
        assert!(
            !names(&on)
                .iter()
                .any(|n| n == "Force Reload" || n == "Toggle Developer Tools")
        );
    }

    #[test]
    fn keeps_ctrl_letters_for_terminals_off_a_mac() {
        let bindings = key_bindings(&AppSettings::default(), Platform::Windows);
        assert_eq!(bindings.len(), 3);
        let mac = key_bindings(&AppSettings::default(), Platform::Mac);
        assert!(mac.len() > 10);
    }

    #[test]
    fn has_windows_file_and_help_items() {
        let menus = app_menus(&AppSettings::default(), Platform::Windows);
        let names: Vec<&str> = menus.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(names, ["File", "Edit", "View", "Window", "Help"]);
        let mac: Vec<String> = app_menus(&AppSettings::default(), Platform::Mac)
            .iter()
            .map(|m| m.name.to_string())
            .collect();
        assert_eq!(mac, ["Nodus", "File", "Edit", "View", "Window", "Help"]);
    }
}
