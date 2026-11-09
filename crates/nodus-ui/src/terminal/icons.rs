//! The lucide icons only the terminal's windows draw (lucide-react 1.49, ISC
//! licence), served beside the shared ones in `components::icon` under their
//! own asset prefix, at the stroke width each place asks for.

use gpui::{Rgba, SharedString, Styled, Svg, px, svg};

/// The icons, by lucide name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermIcon {
    Play,
    RotateCcw,
    Square,
    Folder,
    GitBranch,
    LoaderCircle,
    Mic,
    SquareTerminal,
}

impl TermIcon {
    const ALL: [Self; 8] = [
        Self::Play,
        Self::RotateCcw,
        Self::Square,
        Self::Folder,
        Self::GitBranch,
        Self::LoaderCircle,
        Self::Mic,
        Self::SquareTerminal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::RotateCcw => "rotate-ccw",
            Self::Square => "square",
            Self::Folder => "folder",
            Self::GitBranch => "git-branch",
            Self::LoaderCircle => "loader-circle",
            Self::Mic => "mic",
            Self::SquareTerminal => "square-terminal",
        }
    }

    /// The icon's shapes, inside a 24×24 viewBox.
    pub fn body(self) -> &'static str {
        match self {
            Self::Play => {
                r#"<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>"#
            }
            Self::RotateCcw => r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#,
            Self::Square => r#"<rect width="18" height="18" x="3" y="3" rx="2"/>"#,
            Self::Folder => {
                r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
            }
            Self::GitBranch => {
                r#"<path d="M15 6a9 9 0 0 0-9 9V3"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/>"#
            }
            Self::LoaderCircle => r#"<path d="M21 12a9 9 0 1 1-6.219-8.56"/>"#,
            Self::Mic => {
                r#"<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>"#
            }
            Self::SquareTerminal => {
                r#"<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2"/>"#
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
        format!("icons/terminal/{}@{stroke}.svg", self.name()).into()
    }
}

/// A terminal icon `size` px square, its strokes `stroke` wide, in `color`.
pub fn term_icon(icon: TermIcon, size: f32, stroke: f32, color: impl Into<Rgba>) -> Svg {
    let color: Rgba = color.into();
    svg()
        .path(icon.path(stroke))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// The SVG for an asset path under `icons/terminal/`, if it names one.
pub fn load(path: &str) -> Option<Vec<u8>> {
    let rest = path.strip_prefix("icons/terminal/")?;
    let (name, stroke) = rest.strip_suffix(".svg")?.split_once('@')?;
    let icon = TermIcon::from_name(name)?;
    let stroke = stroke.parse::<f32>().ok()?;
    Some(icon.svg_source(stroke).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_each_icon_at_any_stroke() {
        let svg = String::from_utf8(load("icons/terminal/square@2.4.svg").unwrap()).unwrap();
        assert!(svg.contains(r#"stroke-width="2.4""#) && svg.contains(r#"rx="2""#));
        assert_eq!(TermIcon::Play.path(2.0).as_ref(), "icons/terminal/play@2.svg");
        assert!(load("icons/nodus/x@2.svg").is_none());
        for icon in TermIcon::ALL {
            assert_eq!(TermIcon::from_name(icon.name()), Some(icon));
        }
    }
}
