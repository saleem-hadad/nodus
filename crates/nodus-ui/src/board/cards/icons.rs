//! The lucide icons (lucide-react 1.49, ISC licence) the cards, deck,
//! archive and notes draw beyond the shell's set, served by the app's asset
//! source under `icons/cards/<name>@<stroke>.svg`.

use std::borrow::Cow;

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardIcon {
    Bot,
    GitBranch,
    GitMerge,
    GitFork,
    MessageSquare,
    Maximize2,
    Minimize2,
    AppWindowMac,
    Square,
    Play,
    /// `<Play fill="currentColor">`: the note's ▶ and the mic's stop square.
    PlayFilled,
    SquareFilled,
    RotateCcw,
    LoaderCircle,
    ArrowRight,
    StickyNote,
    Mic,
    SquareTerminal,
    Archive,
    Search,
    X,
    FileDiff,
    GitPullRequest,
    GitPullRequestDraft,
    GitPullRequestClosed,
    ArrowUpRight,
    ChevronDown,
    /// Lucide `network`: the board's coordinator (spec 22).
    Network,
}

impl CardIcon {
    const ALL: [Self; 28] = [
        Self::Bot,
        Self::GitBranch,
        Self::GitMerge,
        Self::GitFork,
        Self::MessageSquare,
        Self::Maximize2,
        Self::Minimize2,
        Self::AppWindowMac,
        Self::Square,
        Self::Play,
        Self::PlayFilled,
        Self::SquareFilled,
        Self::RotateCcw,
        Self::LoaderCircle,
        Self::ArrowRight,
        Self::StickyNote,
        Self::Mic,
        Self::SquareTerminal,
        Self::Archive,
        Self::Search,
        Self::X,
        Self::FileDiff,
        Self::GitPullRequest,
        Self::GitPullRequestDraft,
        Self::GitPullRequestClosed,
        Self::ArrowUpRight,
        Self::ChevronDown,
        Self::Network,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bot => "bot",
            Self::GitBranch => "git-branch",
            Self::GitMerge => "git-merge",
            Self::GitFork => "git-fork",
            Self::MessageSquare => "message-square",
            Self::Maximize2 => "maximize-2",
            Self::Minimize2 => "minimize-2",
            Self::AppWindowMac => "app-window-mac",
            Self::Square => "square",
            Self::Play => "play",
            Self::PlayFilled => "play-filled",
            Self::SquareFilled => "square-filled",
            Self::RotateCcw => "rotate-ccw",
            Self::LoaderCircle => "loader-circle",
            Self::ArrowRight => "arrow-right",
            Self::StickyNote => "sticky-note",
            Self::Mic => "mic",
            Self::SquareTerminal => "square-terminal",
            Self::Archive => "archive",
            Self::Search => "search",
            Self::X => "x",
            Self::FileDiff => "file-diff",
            Self::GitPullRequest => "git-pull-request",
            Self::GitPullRequestDraft => "git-pull-request-draft",
            Self::GitPullRequestClosed => "git-pull-request-closed",
            Self::ArrowUpRight => "arrow-up-right",
            Self::ChevronDown => "chevron-down",
            Self::Network => "network",
        }
    }

    fn filled(self) -> bool {
        matches!(self, Self::PlayFilled | Self::SquareFilled)
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub fn body(self) -> &'static str {
        match self {
            Self::Bot => {
                r#"<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>"#
            }
            Self::GitBranch => {
                r#"<path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/>"#
            }
            Self::GitMerge => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>"#
            }
            Self::GitFork => {
                r#"<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>"#
            }
            Self::MessageSquare => {
                r#"<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/>"#
            }
            Self::Maximize2 => {
                r#"<path d="M15 3h6v6"/><path d="m21 3-7 7"/><path d="m3 21 7-7"/><path d="M9 21H3v-6"/>"#
            }
            Self::Minimize2 => {
                r#"<path d="m14 10 7-7"/><path d="M20 10h-6V4"/><path d="m3 21 7-7"/><path d="M4 14h6v6"/>"#
            }
            Self::AppWindowMac => {
                r#"<rect width="20" height="16" x="2" y="4" rx="2"/><path d="M6 8h.01"/><path d="M10 8h.01"/><path d="M14 8h.01"/>"#
            }
            Self::Square | Self::SquareFilled => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
            Self::Play | Self::PlayFilled => {
                r#"<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>"#
            }
            Self::RotateCcw => r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#,
            Self::LoaderCircle => r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#,
            Self::ArrowRight => r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#,
            Self::StickyNote => {
                r#"<path d="M21 9a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 15 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2z"/><path d="M15 3v5a1 1 0 0 0 1 1h5"/>"#
            }
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::SquareTerminal => {
                r#"<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/>"#
            }
            Self::Archive => {
                r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>"#
            }
            Self::Search => r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#,
            Self::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Self::FileDiff => {
                r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/>"#
            }
            Self::GitPullRequest => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><path d="M6 9v12"/>"#
            }
            Self::GitPullRequestDraft => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M18 6V5"/><path d="M18 11v-1"/><path d="M6 9v12"/>"#
            }
            Self::GitPullRequestClosed => {
                r#"<circle cx="6" cy="6" r="3"/><path d="M6 9v12"/><path d="m21 3-6 6"/><path d="m21 9-6-6"/><path d="M18 11.5V15"/><circle cx="18" cy="18" r="3"/>"#
            }
            Self::ArrowUpRight => r#"<path d="M7 7h10v10"/><path d="M7 17 17 7"/>"#,
            Self::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Self::Network => {
                r#"<rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3"/><path d="M12 12V8"/>"#
            }
        }
    }

    pub fn svg_source(self, stroke: f32) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="{}" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            if self.filled() { "black" } else { "none" },
            self.body()
        )
    }

    pub fn path(self, stroke: f32) -> SharedString {
        format!("icons/cards/{}@{stroke}.svg", self.name()).into()
    }
}

/// The expanded card's resize grip (XCARD-031): the two stripes of
/// `linear-gradient(135deg, … 46% 54% … 72% 80% …)` across an 11px square.
const GRIP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 11 11"><path d="M10.12 0H11V0.88L0.88 11H0V10.12Z"/><path d="M11 4.84V6.6L6.6 11H4.84Z"/></svg>"#;

pub const GRIP_PATH: &str = "icons/cards/grip.svg";

/// Serves `icons/cards/<rest>` for the app's asset source.
pub fn load(rest: &str) -> Option<Cow<'static, [u8]>> {
    if rest == "grip.svg" {
        return Some(Cow::Borrowed(GRIP.as_bytes()));
    }
    let (name, stroke) = rest.strip_suffix(".svg")?.split_once('@')?;
    let icon = CardIcon::ALL.into_iter().find(|i| i.name() == name)?;
    let stroke: f32 = stroke.parse().ok()?;
    Some(Cow::Owned(icon.svg_source(stroke).into_bytes()))
}

/// A lucide icon `size` px square (already scaled), strokes `stroke` wide in
/// its 24-unit box, in `color`.
pub fn card_icon(icon: CardIcon, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(icon.path(stroke))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_every_icon() {
        for icon in CardIcon::ALL {
            let svg = load(&format!("{}@2.2.svg", icon.name())).unwrap_or_else(|| panic!("{}", icon.name()));
            let text = String::from_utf8(svg.into_owned()).unwrap();
            assert!(text.contains(r#"stroke-width="2.2""#));
        }
        assert!(
            String::from_utf8(load("play-filled@2.5.svg").unwrap().into_owned())
                .unwrap()
                .contains(r#"fill="black""#)
        );
        assert!(load("nope@2.svg").is_none());
    }
}
