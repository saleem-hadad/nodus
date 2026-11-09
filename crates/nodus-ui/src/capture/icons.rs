//! The lucide icons only the capture panel draws (lucide-react 1.49, ISC
//! licence), served as `icons/capture/<name>@<stroke>.svg` beside the shared
//! ones. Stroke widths are in the 24-unit viewBox, as `components::icon`'s.

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};

/// The panel's own icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapIcon {
    CircleAlert,
    CornerDownRight,
    FileText,
    Globe,
    LayoutDashboard,
    LoaderCircle,
    Mic,
    Paperclip,
    Scan,
    /// `Square` with `fill="currentColor"` (the mic while recording).
    SquareFilled,
}

impl CapIcon {
    const ALL: [Self; 10] = [
        Self::CircleAlert,
        Self::CornerDownRight,
        Self::FileText,
        Self::Globe,
        Self::LayoutDashboard,
        Self::LoaderCircle,
        Self::Mic,
        Self::Paperclip,
        Self::Scan,
        Self::SquareFilled,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::CircleAlert => "circle-alert",
            Self::CornerDownRight => "corner-down-right",
            Self::FileText => "file-text",
            Self::Globe => "globe",
            Self::LayoutDashboard => "layout-dashboard",
            Self::LoaderCircle => "loader-circle",
            Self::Mic => "mic",
            Self::Paperclip => "paperclip",
            Self::Scan => "scan",
            Self::SquareFilled => "square-filled",
        }
    }

    /// The shapes inside the 24×24 viewBox.
    fn body(self) -> &'static str {
        match self {
            Self::CircleAlert => {
                r#"<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>"#
            }
            Self::CornerDownRight => r#"<path d="m15 10 5 5-5 5"/><path d="M4 4v7a4 4 0 0 0 4 4h12"/>"#,
            Self::FileText => {
                r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
            }
            Self::Globe => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>"#
            }
            Self::LayoutDashboard => {
                r#"<rect width="7" height="9" x="3" y="3" rx="1"/><rect width="7" height="5" x="14" y="3" rx="1"/><rect width="7" height="9" x="14" y="12" rx="1"/><rect width="7" height="5" x="3" y="16" rx="1"/>"#
            }
            Self::LoaderCircle => r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#,
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::Paperclip => {
                r#"<path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"/>"#
            }
            Self::Scan => {
                r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/>"#
            }
            Self::SquareFilled => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }

    /// The whole SVG at a stroke width.
    pub fn svg_source(self, stroke: f32) -> String {
        let fill = if self == Self::SquareFilled { "black" } else { "none" };
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="{fill}" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }

    pub fn path(self, stroke: f32) -> SharedString {
        format!("icons/capture/{}@{stroke}.svg", self.name()).into()
    }
}

/// A capture icon `size` px square, stroke 2 (lucide's default), in `color`.
pub fn cap_icon(icon: CapIcon, size: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg().path(icon.path(2.0)).size(px(size)).flex_none().text_color(color)
}

/// The SVG for an asset path under `icons/capture/`, if it's one of these.
pub fn load(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix("icons/capture/")?;
    let (name, stroke) = rest.strip_suffix(".svg")?.split_once('@')?;
    let icon = CapIcon::from_name(name)?;
    let stroke = stroke.parse::<f32>().ok()?;
    Some(icon.svg_source(stroke).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_every_icon_by_its_path() {
        for icon in CapIcon::ALL {
            let svg = String::from_utf8(load(&icon.path(2.0)).unwrap()).unwrap();
            assert!(svg.contains(r#"stroke-width="2""#), "{}", icon.name());
        }
        let square = String::from_utf8(load("icons/capture/square-filled@2.svg").unwrap()).unwrap();
        assert!(square.contains(r#"fill="black""#));
        assert_eq!(load("icons/nodus/x@2.svg"), None);
        assert_eq!(load("icons/capture/nope@2.svg"), None);
    }
}
