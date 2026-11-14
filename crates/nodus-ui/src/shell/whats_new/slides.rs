//! The slides What's new shows (UPD-098), newest version first. Each release
//! adds a slide or two for what it brings: a short title, one line, and a
//! screenshot of the feature in each theme (`assets/whats-new/`, JPEG,
//! 600×376: the card's picture at 2x). Slides for old versions can go once
//! few people would update from before them.

/// One feature, as What's new shows it.
pub struct Slide {
    /// The version the feature came in.
    pub version: &'static str,
    pub title: &'static str,
    /// One sentence, two lines at most. A shortcut goes in braces as a Mac
    /// writes it, `{⌥⌘G}`, and shows as the platform writes it.
    pub text: &'static str,
    pub light: &'static [u8],
    pub dark: &'static [u8],
}

macro_rules! shot {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/whats-new/", $name, ".jpg"))
    };
}

pub const SLIDES: &[Slide] = &[
    Slide {
        version: "1.0.20",
        title: "Pick the model",
        text: "Choose Claude’s model and how hard it thinks, from the chat or the prompt bar.",
        light: shot!("1.0.20-model-light"),
        dark: shot!("1.0.20-model-dark"),
    },
    Slide {
        version: "1.0.19",
        title: "Claude sees the board",
        text: "Claude looks at your pictures and sketches, then circles, marks and adds to them.",
        light: shot!("1.0.19-see-light"),
        dark: shot!("1.0.19-see-dark"),
    },
    Slide {
        version: "1.0.18",
        title: "Sessions that talk",
        text: "A dotted line joins sessions that message each other, and each message runs along it as a light.",
        light: shot!("1.0.18-talk-light"),
        dark: shot!("1.0.18-talk-dark"),
    },
    Slide {
        version: "1.0.18",
        title: "Watch Claude work",
        text: "Dots wind while Claude works, the line names each step, and replies settle in word by word.",
        light: shot!("1.0.18-working-light"),
        dark: shot!("1.0.18-working-dark"),
    },
    Slide {
        version: "1.0.18",
        title: "Easier to read",
        text: "Code in colour, your messages tinted, and what each edit changed at a glance.",
        light: shot!("1.0.18-colour-light"),
        dark: shot!("1.0.18-colour-dark"),
    },
    Slide {
        version: "1.0.16",
        title: "A smoother chat",
        text: "Replies ease in, tool calls sit on tidy cards, and the box lifts as you type.",
        light: shot!("1.0.16-chat-light"),
        dark: shot!("1.0.16-chat-dark"),
    },
    Slide {
        version: "1.0.16",
        title: "Frame your work",
        text: "Select a few things and press {⌥⌘G}. Claude names the frame, and it all moves together.",
        light: shot!("1.0.16-frames-light"),
        dark: shot!("1.0.16-frames-dark"),
    },
    Slide {
        version: "1.0.16",
        title: "Group anything",
        text: "{⌘G} glues cards, notes and shapes together. Click one to grab them all.",
        light: shot!("1.0.16-groups-light"),
        dark: shot!("1.0.16-groups-dark"),
    },
    Slide {
        version: "1.0.16",
        title: "Talk to Claude",
        text: "The mic now sits in the chat box, right beside Send.",
        light: shot!("1.0.16-mic-light"),
        dark: shot!("1.0.16-mic-dark"),
    },
];

#[cfg(test)]
mod tests {
    use super::SLIDES;

    fn version(v: &str) -> [u32; 3] {
        let parts: Vec<u32> = v.split('.').map(|p| p.parse().unwrap()).collect();
        [parts[0], parts[1], parts[2]]
    }

    /// A JPEG's width and height, from its first start-of-frame marker.
    fn jpeg_size(bytes: &[u8]) -> Option<(u16, u16)> {
        if !bytes.starts_with(&[0xff, 0xd8]) {
            return None;
        }
        let mut at = 2;
        while at + 9 < bytes.len() {
            let (marker, len) = (
                bytes[at + 1],
                u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize,
            );
            if matches!(marker, 0xc0..=0xc2) {
                let h = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
                let w = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]);
                return Some((w, h));
            }
            at += 2 + len;
        }
        None
    }

    #[test]
    fn slides_are_newest_first_short_and_have_a_screenshot_per_theme() {
        assert!(
            SLIDES
                .windows(2)
                .all(|w| version(w[0].version) >= version(w[1].version))
        );
        for slide in SLIDES {
            assert!(slide.title.chars().count() <= 28, "{}", slide.title);
            assert!(slide.text.chars().count() <= 100, "{}", slide.text);
            assert_eq!(jpeg_size(slide.light), Some((600, 376)), "{} light", slide.title);
            assert_eq!(jpeg_size(slide.dark), Some((600, 376)), "{} dark", slide.title);
        }
    }
}
