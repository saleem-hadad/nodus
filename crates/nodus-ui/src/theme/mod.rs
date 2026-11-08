//! The theme: which appearance is in effect, its tokens, the fonts, and the
//! easings (spec 08, Design tokens and UI-001…UI-005).
//!
//! The renderer never chose its theme in 0.9.4: the main process set the
//! OS-level appearance (`nativeTheme.themeSource`) and the CSS followed
//! `prefers-color-scheme`. Here the same happens: [`apply_preference`] sets
//! the app's appearance (so native menus and dialogs follow it too), and each
//! window re-themes when its effective appearance changes.

pub mod fonts;
pub mod motion;
pub mod tokens;

use gpui::{App, Global, Window, WindowAppearance};
use nodus_domain::ThemePreference;

pub use tokens::Tokens;

/// The theme in effect, as a global.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub preference: ThemePreference,
    pub tokens: Tokens,
}

impl Global for Theme {}

/// The tokens in effect.
pub fn tokens(cx: &App) -> Tokens {
    cx.try_global::<Theme>().map_or(Tokens::LIGHT, |t| t.tokens)
}

/// UI-001: light when the setting says light, dark when it says dark, and
/// the OS's appearance when it says system.
pub fn effective_dark(preference: ThemePreference, os: WindowAppearance) -> bool {
    match preference {
        ThemePreference::Light => false,
        ThemePreference::Dark => true,
        ThemePreference::System => is_dark(os),
    }
}

fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// Sets the theme preference: the app's own appearance (`themeSource`), and
/// the tokens for it (UI-004, UI-005). Runs before the window exists, so the
/// first frame is already in the saved theme.
pub fn apply_preference(preference: ThemePreference, cx: &mut App) {
    #[cfg(target_os = "macos")]
    crate::mac::set_app_appearance(preference);
    let dark = effective_dark(preference, cx.window_appearance());
    set(preference, dark, cx);
}

/// Re-themes from a window's effective appearance: on every OS switch while
/// the setting is `system` (UI-002), and when the setting changes.
pub fn sync_window(window: &mut Window, cx: &mut App) {
    let preference = cx
        .try_global::<Theme>()
        .map_or(ThemePreference::System, |t| t.preference);
    let dark = effective_dark(preference, window.appearance());
    set(preference, dark, cx);
    #[cfg(target_os = "macos")]
    crate::mac::set_window_background(window, tokens(cx).bg_window);
    window.refresh();
}

fn set(preference: ThemePreference, dark: bool, cx: &mut App) {
    let tokens = if dark { Tokens::DARK } else { Tokens::LIGHT };
    let changed = cx
        .try_global::<Theme>()
        .is_none_or(|t| t.tokens.dark != dark || t.preference != preference);
    cx.set_global(Theme { preference, tokens });
    if changed {
        sync_component_theme(dark, &tokens, cx);
    }
}

/// gpui-component draws the text fields: give them the app's colours.
fn sync_component_theme(dark: bool, t: &Tokens, cx: &mut App) {
    use gpui::component::theme::{Theme as ComponentTheme, ThemeMode};
    ComponentTheme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    let t = *t;
    let theme = ComponentTheme::global_mut(cx);
    theme.font_family = fonts::UI.into();
    theme.foreground = t.text_primary.into();
    theme.caret = t.text_primary.into();
    theme.muted_foreground = t.text_tertiary.into();
    theme.selection = if dark {
        tokens::rgba(63, 99, 139, 1.0).into()
    } else {
        tokens::rgba(179, 215, 255, 1.0).into()
    };
    theme.background = t.bg_elevated.into();
}
