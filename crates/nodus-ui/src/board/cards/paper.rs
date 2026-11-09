//! Sticky-note paper (NOTEUI-002): the colour's hue mixed 72% into the
//! theme's paper (`color-mix(in srgb, var(--note-hue) 72%, var(--note-paper))`).

use gpui::Rgba;
use nodus_domain::FolderColor;

use crate::components::folder_icon::folder_hues;
use crate::theme::tokens::Tokens;

/// The paper of a note of `color`.
pub fn paper(color: FolderColor, t: &Tokens) -> Rgba {
    let hue = folder_hues(color).front_top;
    let p = t.note_paper;
    let m = |a: f32, b: f32| a * 0.72 + b * 0.28;
    // Both are opaque, so the premultiplied mix is the plain one; CSS
    // resolves it to 8-bit channels.
    let q = |v: f32| (v * 255.0).round() / 255.0;
    Rgba {
        r: q(m(hue.r, p.r)),
        g: q(m(hue.g, p.g)),
        b: q(m(hue.b, p.b)),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::hex;

    #[test]
    fn mixes_the_papers_spec_10_lists() {
        let cases = [
            (FolderColor::Blue, 0x96ddfd, 0x81c8e9),
            (FolderColor::Purple, 0xd7bcf9, 0xc2a7e5),
            (FolderColor::Pink, 0xfcbbd7, 0xe7a6c4),
            (FolderColor::Red, 0xffb0aa, 0xea9b96),
            (FolderColor::Orange, 0xffd49b, 0xeabf87),
            (FolderColor::Yellow, 0xffea9f, 0xead58b),
            (FolderColor::Green, 0xb1e9b8, 0x9cd4a4),
            (FolderColor::Graphite, 0xcfcfd3, 0xbababf),
        ];
        for (color, light, dark) in cases {
            assert_eq!(paper(color, &Tokens::LIGHT), hex(light), "{color:?} light");
            assert_eq!(paper(color, &Tokens::DARK), hex(dark), "{color:?} dark");
        }
    }
}
