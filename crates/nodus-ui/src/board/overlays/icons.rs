//! The lucide icons only the board's overlays draw (lucide-react 1.49, ISC
//! licence), served beside the shared ones in `components::icon` under their
//! own asset prefix, at the stroke width each place asks for.

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};

/// The icons, by lucide name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OvIcon {
    MousePointer2,
    MessageCircleMore,
    MoveUpRight,
    Square,
    Type,
    Eraser,
    Mic,
    ArrowUp,
    CornerDownRight,
    Circle,
    CheckCircle2,
    AlertCircle,
    RotateCcw,
    Diamond,
    Smile,
    Image,
}

impl OvIcon {
    const ALL: [Self; 16] = [
        Self::MousePointer2,
        Self::MessageCircleMore,
        Self::MoveUpRight,
        Self::Square,
        Self::Type,
        Self::Eraser,
        Self::Mic,
        Self::ArrowUp,
        Self::CornerDownRight,
        Self::Circle,
        Self::CheckCircle2,
        Self::AlertCircle,
        Self::RotateCcw,
        Self::Diamond,
        Self::Smile,
        Self::Image,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::MousePointer2 => "mouse-pointer-2",
            Self::MessageCircleMore => "message-circle-more",
            Self::MoveUpRight => "move-up-right",
            Self::Square => "square",
            Self::Type => "type",
            Self::Eraser => "eraser",
            Self::Mic => "mic",
            Self::ArrowUp => "arrow-up",
            Self::CornerDownRight => "corner-down-right",
            Self::Circle => "circle",
            Self::CheckCircle2 => "circle-check",
            Self::AlertCircle => "circle-alert",
            Self::RotateCcw => "rotate-ccw",
            Self::Diamond => "diamond",
            Self::Smile => "smile",
            Self::Image => "image",
        }
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub fn body(self) -> &'static str {
        match self {
            Self::MousePointer2 => {
                r#"<path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z"/>"#
            }
            Self::MessageCircleMore => {
                r#"<path d="M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0-4.777-4.719"/><path d="M8 12h.01"/><path d="M12 12h.01"/><path d="M16 12h.01"/>"#
            }
            Self::MoveUpRight => r#"<path d="M13 5H19V11"/><path d="M19 5L5 19"/>"#,
            Self::Square => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
            Self::Type => {
                r#"<path d="M12 4v16"/><path d="M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2"/><path d="M9 20h6"/>"#
            }
            Self::Eraser => {
                r#"<path d="M21 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.829 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21"/><path d="m5.082 11.09 8.828 8.828"/>"#
            }
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::ArrowUp => r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#,
            Self::CornerDownRight => r#"<path d="m15 10 5 5-5 5"/><path d="M4 4v7a4 4 0 0 0 4 4h12"/>"#,
            Self::Circle => r#"<circle cx="12" cy="12" r="10"/>"#,
            Self::CheckCircle2 => r#"<circle cx="12" cy="12" r="10"/><path d="m16 9-5.5 5.5L8 12"/>"#,
            Self::AlertCircle => {
                r#"<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>"#
            }
            Self::RotateCcw => r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#,
            Self::Diamond => {
                r#"<path d="M2.7 10.3a2.41 2.41 0 0 0 0 3.41l7.59 7.59a2.41 2.41 0 0 0 3.41 0l7.59-7.59a2.41 2.41 0 0 0 0-3.41l-7.59-7.59a2.41 2.41 0 0 0-3.41 0Z"/>"#
            }
            Self::Smile => {
                r#"<circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/>"#
            }
            Self::Image => {
                r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
            }
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }

    pub fn path(self, stroke: f32) -> SharedString {
        format!("icons/overlays/{}@{stroke}.svg", self.name()).into()
    }
}

/// An overlay icon `size` px square, its strokes `stroke` wide, in `color`.
pub fn ov_icon(icon: OvIcon, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(icon.path(stroke))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// The SVG for an asset path under `icons/overlays/`, if it names one.
pub fn load(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix("icons/overlays/")?;
    let (name, stroke) = rest.strip_suffix(".svg")?.split_once('@')?;
    let icon = OvIcon::from_name(name)?;
    let stroke: f32 = stroke.parse().ok()?;
    Some(
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            icon.body()
        )
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_each_icon() {
        for icon in OvIcon::ALL {
            let svg = String::from_utf8(load(&icon.path(2.2)).unwrap()).unwrap();
            assert!(svg.contains(r#"stroke-width="2.2""#));
        }
        assert!(load("icons/nodus/x@2.svg").is_none());
    }
}
