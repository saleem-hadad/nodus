//! The lucide icons the actions rail and dialog, the git controls and the
//! Changes window, and the mics draw (lucide-react 1.49.0, ISC licence; spec
//! 05's "Icons used"), served beside the shared ones in `components::icon`
//! under their own asset prefix, at the stroke width each place asks for.
//! The 26 action icons are `nodus_domain::ActionIcon`'s, in its order.

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};
use nodus_domain::ActionIcon;

/// The asset prefix these icons are served under.
const PREFIX: &str = "icons/spec05/";

/// The icons, by lucide name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Zap,
    RefreshCw,
    GitMerge,
    GitPullRequest,
    GitCommitHorizontal,
    Upload,
    Download,
    Archive,
    BrushCleaning,
    Eraser,
    Sparkles,
    WandSparkles,
    TestTube,
    Bug,
    Rocket,
    Package,
    FileText,
    BookOpen,
    ListChecks,
    Search,
    ShieldCheck,
    Hammer,
    SquareTerminal,
    Gauge,
    Lightbulb,
    MessageSquare,
    Globe,
    Plus,
    Play,
    Pencil,
    LoaderCircle,
    Mic,
    Square,
    X,
    CircleAlert,
    RotateCcw,
    Settings,
    GitBranch,
    FileDiff,
    SquarePlus,
    SquareMinus,
    SquareArrowRight,
    SquareDot,
    TriangleAlert,
    /// Not lucide: the corner grip Chromium draws on a vertically resizable
    /// textarea (the action dialog's description).
    ResizeGrip,
}

impl Glyph {
    pub const ALL: [Self; 45] = [
        Self::Zap,
        Self::RefreshCw,
        Self::GitMerge,
        Self::GitPullRequest,
        Self::GitCommitHorizontal,
        Self::Upload,
        Self::Download,
        Self::Archive,
        Self::BrushCleaning,
        Self::Eraser,
        Self::Sparkles,
        Self::WandSparkles,
        Self::TestTube,
        Self::Bug,
        Self::Rocket,
        Self::Package,
        Self::FileText,
        Self::BookOpen,
        Self::ListChecks,
        Self::Search,
        Self::ShieldCheck,
        Self::Hammer,
        Self::SquareTerminal,
        Self::Gauge,
        Self::Lightbulb,
        Self::MessageSquare,
        Self::Globe,
        Self::Plus,
        Self::Play,
        Self::Pencil,
        Self::LoaderCircle,
        Self::Mic,
        Self::Square,
        Self::X,
        Self::CircleAlert,
        Self::RotateCcw,
        Self::Settings,
        Self::GitBranch,
        Self::FileDiff,
        Self::SquarePlus,
        Self::SquareMinus,
        Self::SquareArrowRight,
        Self::SquareDot,
        Self::TriangleAlert,
        Self::ResizeGrip,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Zap => "zap",
            Self::RefreshCw => "refresh-cw",
            Self::GitMerge => "git-merge",
            Self::GitPullRequest => "git-pull-request",
            Self::GitCommitHorizontal => "git-commit-horizontal",
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Archive => "archive",
            Self::BrushCleaning => "brush-cleaning",
            Self::Eraser => "eraser",
            Self::Sparkles => "sparkles",
            Self::WandSparkles => "wand-sparkles",
            Self::TestTube => "test-tube",
            Self::Bug => "bug",
            Self::Rocket => "rocket",
            Self::Package => "package",
            Self::FileText => "file-text",
            Self::BookOpen => "book-open",
            Self::ListChecks => "list-checks",
            Self::Search => "search",
            Self::ShieldCheck => "shield-check",
            Self::Hammer => "hammer",
            Self::SquareTerminal => "square-terminal",
            Self::Gauge => "gauge",
            Self::Lightbulb => "lightbulb",
            Self::MessageSquare => "message-square",
            Self::Globe => "globe",
            Self::Plus => "plus",
            Self::Play => "play",
            Self::Pencil => "pencil",
            Self::LoaderCircle => "loader-circle",
            Self::Mic => "mic",
            Self::Square => "square",
            Self::X => "x",
            Self::CircleAlert => "circle-alert",
            Self::RotateCcw => "rotate-ccw",
            Self::Settings => "settings",
            Self::GitBranch => "git-branch",
            Self::FileDiff => "file-diff",
            Self::SquarePlus => "square-plus",
            Self::SquareMinus => "square-minus",
            Self::SquareArrowRight => "square-arrow-right",
            Self::SquareDot => "square-dot",
            Self::TriangleAlert => "triangle-alert",
            Self::ResizeGrip => "resize-grip",
        }
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub const fn body(self) -> &'static str {
        match self {
            Self::Zap => {
                r#"<path d="M15.914 4a1.5 1.5 0 00-2.474-1.561l-9 9A1.5 1.5 0 005.5 14h4.002a.5.5 0 01.471.666L8.086 20a1.5 1.5 0 002.475 1.56l9-9A1.5 1.5 0 0018.5 10h-3.997a.5.5 0 01-.472-.667z"/>"#
            }
            Self::RefreshCw => {
                r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>"#
            }
            Self::GitMerge => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>"#
            }
            Self::GitPullRequest => {
                r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" x2="6" y1="9" y2="21"/>"#
            }
            Self::GitCommitHorizontal => {
                r#"<circle cx="12" cy="12" r="3"/><line x1="3" x2="9" y1="12" y2="12"/><line x1="15" x2="21" y1="12" y2="12"/>"#
            }
            Self::Upload => {
                r#"<path d="M12 3v12"/><path d="m17 8-5-5-5 5"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>"#
            }
            Self::Download => {
                r#"<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>"#
            }
            Self::Archive => {
                r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>"#
            }
            Self::BrushCleaning => {
                r#"<path d="m16 22-1-4"/><path d="M19 14a1 1 0 0 0 1-1v-1a2 2 0 0 0-2-2h-3a1 1 0 0 1-1-1V4a2 2 0 0 0-4 0v5a1 1 0 0 1-1 1H6a2 2 0 0 0-2 2v1a1 1 0 0 0 1 1"/><path d="M19 14H5l-1.973 6.767A1 1 0 0 0 4 22h16a1 1 0 0 0 .973-1.233z"/><path d="m8 22 1-4"/>"#
            }
            Self::Eraser => {
                r#"<path d="M21 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.829 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21"/><path d="m5.082 11.09 8.828 8.828"/>"#
            }
            Self::Sparkles => {
                r#"<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/>"#
            }
            Self::WandSparkles => {
                r#"<path d="m21.64 3.64-1.28-1.28a1.21 1.21 0 0 0-1.72 0L2.36 18.64a1.21 1.21 0 0 0 0 1.72l1.28 1.28a1.2 1.2 0 0 0 1.72 0L21.64 5.36a1.2 1.2 0 0 0 0-1.72"/><path d="m14 7 3 3"/><path d="M5 6v4"/><path d="M19 14v4"/><path d="M10 2v2"/><path d="M7 8H3"/><path d="M21 16h-4"/><path d="M11 3H9"/>"#
            }
            Self::TestTube => {
                r#"<path d="M14.5 2v17.5c0 1.4-1.1 2.5-2.5 2.5c-1.4 0-2.5-1.1-2.5-2.5V2"/><path d="M8.5 2h7"/><path d="M14.5 16h-5"/>"#
            }
            Self::Bug => {
                r#"<path d="M12 20v-9"/><path d="M14 7a4 4 0 0 1 4 4v3a6 6 0 0 1-12 0v-3a4 4 0 0 1 4-4z"/><path d="M14.12 3.88 16 2"/><path d="M21 21a4 4 0 0 0-3.81-4"/><path d="M21 5a4 4 0 0 1-3.55 3.97"/><path d="M22 13h-4"/><path d="M3 21a4 4 0 0 1 3.81-4"/><path d="M3 5a4 4 0 0 0 3.55 3.97"/><path d="M6 13H2"/><path d="m8 2 1.88 1.88"/><path d="M9 7.13V6a3 3 0 1 1 6 0v1.13"/>"#
            }
            Self::Rocket => {
                r#"<path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5"/><path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09"/><path d="M9 12a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.4 22.4 0 0 1-4 2z"/><path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 .05 5 .05"/>"#
            }
            Self::Package => {
                r#"<path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z"/><path d="M12 22V12"/><polyline points="3.29 7 12 12 20.71 7"/><path d="m7.5 4.27 9 5.15"/>"#
            }
            Self::FileText => {
                r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
            }
            Self::BookOpen => {
                r#"<path d="M12 5v16"/><path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z"/>"#
            }
            Self::ListChecks => {
                r#"<path d="M13 5h8"/><path d="M13 12h8"/><path d="M13 19h8"/><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/>"#
            }
            Self::Search => r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#,
            Self::ShieldCheck => {
                r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>"#
            }
            Self::Hammer => {
                r#"<path d="m15 12-9.373 9.373a1 1 0 0 1-3.001-3L12 9"/><path d="m18 15 4-4"/><path d="m21.5 11.5-1.914-1.914A2 2 0 0 1 19 8.172v-.344a2 2 0 0 0-.586-1.414l-1.657-1.657A6 6 0 0 0 12.516 3H9l1.243 1.243A6 6 0 0 1 12 8.485V10l2 2h1.172a2 2 0 0 1 1.414.586L18.5 14.5"/>"#
            }
            Self::SquareTerminal => {
                r#"<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/>"#
            }
            Self::Gauge => r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#,
            Self::Lightbulb => {
                r#"<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"/><path d="M9 18h6"/><path d="M10 22h4"/>"#
            }
            Self::MessageSquare => {
                r#"<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/>"#
            }
            Self::Globe => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>"#
            }
            Self::Plus => r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#,
            Self::Play => {
                r#"<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>"#
            }
            Self::Pencil => {
                r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
            }
            Self::LoaderCircle => r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#,
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::Square => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
            Self::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Self::CircleAlert => {
                r#"<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>"#
            }
            Self::RotateCcw => r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#,
            Self::Settings => {
                r#"<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/>"#
            }
            Self::GitBranch => {
                r#"<path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/>"#
            }
            Self::FileDiff => {
                r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/>"#
            }
            Self::SquarePlus => {
                r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 12h8"/><path d="M12 8v8"/>"#
            }
            Self::SquareMinus => r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 12h8"/>"#,
            Self::SquareArrowRight => {
                r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M8 12h8"/><path d="m12 16 4-4-4-4"/>"#
            }
            Self::SquareDot => r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="12" cy="12" r="1"/>"#,
            Self::TriangleAlert => {
                r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#
            }
            // Two strokes rising to the right, 7 and 3 units of a 10px box.
            Self::ResizeGrip => r#"<path d="M4.8 20.4 21.4 3.8"/><path d="M14.4 20.4 21.4 13.4"/>"#,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.name() == name)
    }

    /// An action's icon (ACT-081: an unknown name draws `zap`).
    pub fn action(name: &str) -> Self {
        let icon = ActionIcon::parse(name).unwrap_or(ActionIcon::DEFAULT);
        Self::from_name(icon.as_str()).unwrap_or(Self::Zap)
    }

    /// The asset path GPUI loads it by; `filled` draws its shapes filled
    /// with no stroke (the recording mic's square).
    pub fn path(self, stroke: f32, filled: bool) -> SharedString {
        let fill = if filled { "-filled" } else { "" };
        format!("{PREFIX}{}@{stroke}{fill}.svg", self.name()).into()
    }

    /// The whole SVG.
    pub fn svg_source(self, stroke: f32, filled: bool) -> String {
        let (fill, stroke) = if filled { ("black", 0.0) } else { ("none", stroke) };
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="{fill}" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }
}

/// Serves `icons/spec05/<name>@<stroke>[-filled].svg`.
pub fn load(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix(PREFIX)?.strip_suffix(".svg")?;
    let (rest, filled) = match rest.strip_suffix("-filled") {
        Some(rest) => (rest, true),
        None => (rest, false),
    };
    let (name, stroke) = rest.split_once('@')?;
    let glyph = Glyph::from_name(name)?;
    let stroke = stroke.parse::<f32>().ok()?;
    Some(glyph.svg_source(stroke, filled).into_bytes())
}

/// A lucide icon `size` px square, its strokes `stroke` wide (in the 24-unit
/// viewBox), in `color`.
pub fn glyph(glyph: Glyph, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(glyph.path(stroke, false))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A lucide icon drawn filled, with no stroke.
pub fn filled_glyph(glyph: Glyph, size: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(glyph.path(0.0, true))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_every_action_icon_in_order() {
        // ACT-003: the 26, in ACTION_ICONS order, first in the table.
        for (i, icon) in ActionIcon::ALL.iter().enumerate() {
            assert_eq!(Glyph::ALL[i].name(), icon.as_str());
        }
        assert_eq!(Glyph::action("not-an-icon"), Glyph::Zap);
        assert_eq!(Glyph::action("gauge"), Glyph::Gauge);
    }

    #[test]
    fn serves_icons_by_path() {
        let svg = String::from_utf8(load("icons/spec05/git-branch@2.2.svg").unwrap()).unwrap();
        assert!(svg.contains(r#"stroke-width="2.2""#) && svg.contains("M15 6a9 9 0 0 0-9 9V3"));
        let filled = String::from_utf8(load(&Glyph::Square.path(0.0, true)).unwrap()).unwrap();
        assert!(filled.contains(r#"fill="black""#) && filled.contains(r#"stroke-width="0""#));
        assert!(load("icons/spec05/nope@2.svg").is_none());
        assert!(load("icons/nodus/x@2.svg").is_none());
    }
}
