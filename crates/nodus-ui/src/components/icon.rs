//! The lucide icons the app draws (lucide-react 1.49, ISC licence), at the
//! stroke width each place asks for: an icon's stroke is in its 24-unit
//! viewBox, so it's part of the drawing GPUI rasterizes, and each width is its
//! own asset (`icons/nodus/<name>@<width>.svg`).

use std::borrow::Cow;

use gpui::{AssetSource, Rgba, SharedString, Styled, Svg, px, svg};

/// The icons, by lucide name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Menu,
    Plus,
    X,
    SquareTerminal,
    RotateCw,
    Target,
    FolderOpen,
    Pencil,
    FolderInput,
    Search,
    Archive,
    ArchiveRestore,
    Trash2,
    Check,
    Folders,
    FolderMinus,
    FolderPlus,
    Minus,
    Maximize,
    Maximize2,
    Minimize2,
    AppWindowMac,
    Ban,
    Bot,
    Eye,
    Frame,
    Group,
    LayoutDashboard,
    Loader2,
    PencilLine,
    Play,
    RefreshCw,
    Sparkles,
    Square,
    SquareDashed,
    StickyNote,
    Ungroup,
    MessageSquareWarning,
    ExternalLink,
    CircleAlert,
    Shield,
    Bell,
    Globe,
    SunMoon,
    Mic,
    Download,
    Server,
}

impl Icon {
    const ALL: [Self; 50] = [
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ChevronDown,
        Self::Menu,
        Self::Plus,
        Self::X,
        Self::SquareTerminal,
        Self::RotateCw,
        Self::Target,
        Self::FolderOpen,
        Self::Pencil,
        Self::FolderInput,
        Self::Search,
        Self::Archive,
        Self::ArchiveRestore,
        Self::Trash2,
        Self::Check,
        Self::Folders,
        Self::FolderMinus,
        Self::FolderPlus,
        Self::Minus,
        Self::Maximize,
        Self::Maximize2,
        Self::Minimize2,
        Self::AppWindowMac,
        Self::Ban,
        Self::Bot,
        Self::Eye,
        Self::Frame,
        Self::Group,
        Self::LayoutDashboard,
        Self::Loader2,
        Self::PencilLine,
        Self::Play,
        Self::RefreshCw,
        Self::Sparkles,
        Self::Square,
        Self::SquareDashed,
        Self::StickyNote,
        Self::Ungroup,
        Self::MessageSquareWarning,
        Self::ExternalLink,
        Self::CircleAlert,
        Self::Shield,
        Self::Bell,
        Self::Globe,
        Self::SunMoon,
        Self::Mic,
        Self::Download,
        Self::Server,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ChevronLeft => "chevron-left",
            Self::ChevronRight => "chevron-right",
            Self::ChevronDown => "chevron-down",
            Self::Menu => "menu",
            Self::Plus => "plus",
            Self::X => "x",
            Self::SquareTerminal => "square-terminal",
            Self::RotateCw => "rotate-cw",
            Self::Target => "target",
            Self::FolderOpen => "folder-open",
            Self::Pencil => "pencil",
            Self::FolderInput => "folder-input",
            Self::Search => "search",
            Self::Archive => "archive",
            Self::ArchiveRestore => "archive-restore",
            Self::Trash2 => "trash-2",
            Self::Check => "check",
            Self::Folders => "folders",
            Self::FolderMinus => "folder-minus",
            Self::FolderPlus => "folder-plus",
            Self::Minus => "minus",
            Self::Maximize => "maximize",
            Self::Maximize2 => "maximize-2",
            Self::Minimize2 => "minimize-2",
            Self::AppWindowMac => "app-window-mac",
            Self::Ban => "ban",
            Self::Bot => "bot",
            Self::Eye => "eye",
            Self::Frame => "frame",
            Self::Group => "group",
            Self::LayoutDashboard => "layout-dashboard",
            Self::Loader2 => "loader-circle",
            Self::PencilLine => "pencil-line",
            Self::Play => "play",
            Self::RefreshCw => "refresh-cw",
            Self::Sparkles => "sparkles",
            Self::Square => "square",
            Self::SquareDashed => "square-dashed",
            Self::StickyNote => "sticky-note",
            Self::MessageSquareWarning => "message-square-warning",
            Self::ExternalLink => "external-link",
            Self::CircleAlert => "circle-alert",
            Self::Shield => "shield",
            Self::Bell => "bell",
            Self::Globe => "globe",
            Self::SunMoon => "sun-moon",
            Self::Mic => "mic",
            Self::Download => "download",
            Self::Server => "server",
            Self::Ungroup => "ungroup",
        }
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub fn body(self) -> &'static str {
        match self {
            Self::ChevronLeft => r#"<path d="m15 18-6-6 6-6"/>"#,
            Self::ChevronRight => r#"<path d="m9 18 6-6-6-6"/>"#,
            Self::ChevronDown => r#"<path d="m6 9 6 6 6-6"/>"#,
            Self::Menu => r#"<path d="M4 5h16"/><path d="M4 12h16"/><path d="M4 19h16"/>"#,
            Self::Plus => r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#,
            Self::X => r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#,
            Self::SquareTerminal => {
                r#"<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/>"#
            }
            Self::RotateCw => r#"<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>"#,
            Self::Target => {
                r#"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/>"#
            }
            Self::FolderOpen => {
                r#"<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>"#
            }
            Self::Pencil => {
                r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
            }
            Self::FolderInput => {
                r#"<path d="M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1"/><path d="M2 13h10"/><path d="m9 16 3-3-3-3"/>"#
            }
            Self::Search => r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#,
            Self::Archive => {
                r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>"#
            }
            Self::ArchiveRestore => {
                r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h2"/><path d="M20 8v11a2 2 0 0 1-2 2h-2"/><path d="m9 15 3-3 3 3"/><path d="M12 12v9"/>"#
            }
            Self::Trash2 => {
                r#"<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"#
            }
            Self::Check => r#"<path d="M20 6 9 17l-5-5"/>"#,
            Self::Folders => {
                r#"<path d="M20 5a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h2.5a1.5 1.5 0 0 1 1.2.6l.6.8a1.5 1.5 0 0 0 1.2.6z"/><path d="M3 8.268a2 2 0 0 0-1 1.738V19a2 2 0 0 0 2 2h11a2 2 0 0 0 1.732-1"/>"#
            }
            Self::FolderMinus => {
                r#"<path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
            }
            Self::FolderPlus => {
                r#"<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
            }
            Self::Minus => r#"<path d="M5 12h14"/>"#,
            Self::Maximize => {
                r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#
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
            Self::Ban => r#"<circle cx="12" cy="12" r="10"/><path d="M4.929 4.929 19.07 19.071"/>"#,
            Self::Bot => {
                r#"<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>"#
            }
            Self::Eye => {
                r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"#
            }
            Self::Frame => {
                r#"<line x1="22" x2="2" y1="6" y2="6"/><line x1="22" x2="2" y1="18" y2="18"/><line x1="6" x2="6" y1="2" y2="22"/><line x1="18" x2="18" y1="2" y2="22"/>"#
            }
            Self::Group => {
                r#"<path d="M3 7V5c0-1.1.9-2 2-2h2"/><path d="M17 3h2c1.1 0 2 .9 2 2v2"/><path d="M21 17v2c0 1.1-.9 2-2 2h-2"/><path d="M7 21H5c-1.1 0-2-.9-2-2v-2"/><rect width="7" height="5" x="7" y="7" rx="1"/><rect width="7" height="5" x="10" y="12" rx="1"/>"#
            }
            Self::LayoutDashboard => {
                r#"<rect width="7" height="9" x="3" y="3" rx="1"/><rect width="7" height="5" x="14" y="3" rx="1"/><rect width="7" height="9" x="14" y="12" rx="1"/><rect width="7" height="5" x="3" y="16" rx="1"/>"#
            }
            Self::Loader2 => r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#,
            Self::PencilLine => {
                r#"<path d="M13 21h8"/><path d="m15 5 4 4"/><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/>"#
            }
            Self::Play => {
                r#"<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>"#
            }
            Self::RefreshCw => {
                r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>"#
            }
            Self::Sparkles => {
                r#"<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/>"#
            }
            Self::Square => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
            Self::SquareDashed => {
                r#"<path d="M5 3a2 2 0 0 0-2 2"/><path d="M19 3a2 2 0 0 1 2 2"/><path d="M21 19a2 2 0 0 1-2 2"/><path d="M5 21a2 2 0 0 1-2-2"/><path d="M9 3h1"/><path d="M9 21h1"/><path d="M14 3h1"/><path d="M14 21h1"/><path d="M3 9v1"/><path d="M21 9v1"/><path d="M3 14v1"/><path d="M21 14v1"/>"#
            }
            Self::StickyNote => {
                r#"<path d="M21 9a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 15 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2z"/><path d="M15 3v5a1 1 0 0 0 1 1h5"/>"#
            }
            Self::Ungroup => {
                r#"<rect x="11" y="14" width="10" height="7" rx="2"/><rect x="3" y="3" width="10" height="7" rx="2"/>"#
            }
            Self::MessageSquareWarning => {
                r#"<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/><path d="M12 15h.01"/><path d="M12 7v4"/>"#
            }
            Self::ExternalLink => {
                r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#
            }
            Self::CircleAlert => r#"<circle cx="12" cy="12" r="10"/><path d="M12 8v4"/><path d="M12 16h.01"/>"#,
            Self::Shield => {
                r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#
            }
            Self::Globe => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>"#
            }
            Self::Bell => {
                r#"<path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326"/>"#
            }
            Self::SunMoon => {
                r#"<path d="M12 8a2.83 2.83 0 0 0 4 4 4 4 0 1 1-4-4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.9 4.9 1.4 1.4"/><path d="m17.7 17.7 1.4 1.4"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.3 17.7-1.4 1.4"/><path d="m19.1 4.9-1.4 1.4"/>"#
            }
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::Download => {
                r#"<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>"#
            }
            Self::Server => {
                r#"<rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>"#
            }
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }

    /// The whole SVG at a stroke width.
    pub fn svg_source(self, stroke: f32) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }

    /// The asset path GPUI loads it by.
    pub fn path(self, stroke: f32) -> SharedString {
        format!("icons/nodus/{}@{stroke}.svg", self.name()).into()
    }
}

/// A lucide icon `size` px square, its strokes `stroke` wide (in the 24-unit
/// viewBox), in `color`.
pub fn icon(icon: Icon, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(icon.path(stroke))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// The app's assets: the icons above, then gpui-kit's own for the
/// components it draws.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(svg) = crate::views::icons::load(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        if let Some(rest) = path.strip_prefix("icons/nodus/")
            && let Some((name, stroke)) = rest.strip_suffix(".svg").and_then(|r| r.split_once('@'))
            && let (Some(icon), Ok(stroke)) = (Icon::from_name(name), stroke.parse::<f32>())
        {
            return Ok(Some(Cow::Owned(icon.svg_source(stroke).into_bytes())));
        }
        if let Some(rest) = path.strip_prefix("icons/cards/") {
            return Ok(crate::board::cards::icons::load(rest));
        }
        if let Some(svg) = crate::terminal::icons::load(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        if let Some(svg) = crate::capture::icons::load(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        if let Some(svg) = crate::actions::icons::load(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        if let Some(svg) = crate::board::overlays::icons::load(path) {
            return Ok(Some(Cow::Owned(svg)));
        }
        gpui::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        gpui::assets::Assets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_each_icon_at_any_stroke() {
        let svg = Assets.load("icons/nodus/chevron-left@2.2.svg").unwrap().unwrap();
        let text = String::from_utf8(svg.into_owned()).unwrap();
        assert!(text.contains(r#"stroke-width="2.2""#));
        assert!(text.contains("m15 18-6-6 6-6"));
        assert_eq!(Icon::Trash2.path(2.0).as_ref(), "icons/nodus/trash-2@2.svg");
        for icon in Icon::ALL {
            assert_eq!(Icon::from_name(icon.name()), Some(icon));
        }
    }
}
