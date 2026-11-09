//! The lucide icons only views draw (lucide-react, ISC licence), served
//! beside the shared ones in `components::icon` under their own asset prefix:
//! `MoreHorizontal` (⋯, lucide's `ellipsis`) and `Wand2` (`wand-sparkles`).

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};

/// The icons, by lucide name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewIcon {
    Ellipsis,
    WandSparkles,
}

impl ViewIcon {
    const ALL: [Self; 2] = [Self::Ellipsis, Self::WandSparkles];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ellipsis => "ellipsis",
            Self::WandSparkles => "wand-sparkles",
        }
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub fn body(self) -> &'static str {
        match self {
            Self::Ellipsis => {
                r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#
            }
            Self::WandSparkles => {
                r#"<path d="m21.64 3.64-1.28-1.28a1.21 1.21 0 0 0-1.72 0L2.36 18.64a1.21 1.21 0 0 0 0 1.72l1.28 1.28a1.2 1.2 0 0 0 1.72 0L21.64 5.36a1.2 1.2 0 0 0 0-1.72"/><path d="m14 7 3 3"/><path d="M5 6v4"/><path d="M19 14v4"/><path d="M10 2v2"/><path d="M7 8H3"/><path d="M21 16h-4"/><path d="M11 3H9"/>"#
            }
        }
    }

    /// The asset path GPUI loads it by, at a stroke width.
    pub fn path(self, stroke: f32) -> SharedString {
        format!("icons/views/{}@{stroke}.svg", self.name()).into()
    }
}

/// The SVG for an asset path under `icons/views/`, if it is one of these.
pub fn load(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix("icons/views/")?.strip_suffix(".svg")?;
    let (name, stroke) = rest.split_once('@')?;
    let stroke: f32 = stroke.parse().ok()?;
    let icon = ViewIcon::ALL.into_iter().find(|i| i.name() == name)?;
    Some(
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            icon.body()
        )
        .into_bytes(),
    )
}

/// One of these icons, `size` px square.
pub fn view_icon(icon: ViewIcon, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
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
    fn serves_both_icons() {
        let svg = String::from_utf8(load("icons/views/ellipsis@2.svg").unwrap()).unwrap();
        assert!(svg.contains(r#"cx="19""#) && svg.contains(r#"stroke-width="2""#));
        assert!(load("icons/views/wand-sparkles@2.svg").is_some());
        assert!(load("icons/views/nope@2.svg").is_none());
        assert!(load("icons/nodus/x@2.svg").is_none());
    }
}
