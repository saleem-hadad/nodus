//! `src/main/screenContext.test.ts`, case by case, then the rest of what
//! comes from the screen, with synthetic images, a fake clipboard and a
//! stand-in screenshot tool. The user's own screen and clipboard are never read.

use std::sync::Mutex;

use image::{Rgba, RgbaImage};
use nodus_domain::TextAttachment;

use super::*;

#[test]
fn parse_app_info_reads_the_name_and_bundle_id_from_lsappinfo() {
    let out = [
        "\"Safari\" ASN:0x0-0x1a01a: (in front) ",
        "    bundleID=\"com.apple.Safari\"",
        "    bundle path=[ NULL ] ",
        " !cgsConnection !signalled type=[ NULL ]",
    ]
    .join("\n");
    assert_eq!(
        parse_app_info(&out),
        Some(FrontApp {
            name: "Safari".into(),
            bundle_id: "com.apple.Safari".into()
        })
    );
}

#[test]
fn parse_app_info_falls_back_to_the_bundle_id_when_there_is_no_name() {
    assert_eq!(
        parse_app_info("[ NULL ] ASN:0x0-0x1:\n    bundleID=\"com.mitchellh.ghostty\""),
        Some(FrontApp {
            name: "ghostty".into(),
            bundle_id: "com.mitchellh.ghostty".into()
        })
    );
}

#[test]
fn parse_app_info_has_nothing_without_a_bundle_id() {
    assert_eq!(parse_app_info("[ NULL ] ASN:0x0-0x1:\n    bundleID=[ NULL ]"), None);
}

/// CAP-046: the regexes' edges.
#[test]
fn parse_app_info_at_its_edges() {
    // An empty id is skipped for the next one.
    let out = "bundleID=\"\" x bundleID=\"com.example.app\"";
    assert_eq!(parse_app_info(out).unwrap().bundle_id, "com.example.app");
    // The name may sit on a later line, after spaces, but needs `ASN:` after it.
    let out = "junk\n  \"Some App\"   ASN:0x1:\n bundleID=\"com.example.app\"";
    assert_eq!(parse_app_info(out).unwrap().name, "Some App");
    let out = "\"Some App\"ASN:0x1:\n bundleID=\"com.example.app\"";
    assert_eq!(parse_app_info(out).unwrap().name, "app");
    let out = "x \"Some App\" ASN:0x1:\n bundleID=\"com.example.app\"";
    assert_eq!(parse_app_info(out).unwrap().name, "app");
    // An id with no dots is its own name.
    assert_eq!(parse_app_info("bundleID=\"solo\"").unwrap().name, "solo");
}

#[test]
fn tab_script_asks_safari_for_its_front_document() {
    assert_eq!(
        tab_script("com.apple.Safari").as_deref(),
        Some(
            "tell application id \"com.apple.Safari\" to tell front document to return (URL as text) & linefeed & (name as text)"
        )
    );
}

#[test]
fn tab_script_asks_a_chromium_browser_for_its_active_tab() {
    assert!(
        tab_script("company.thebrowser.Browser")
            .unwrap()
            .contains("tell active tab of front window")
    );
    assert_eq!(
        tab_script("com.google.Chrome").as_deref(),
        Some(
            "tell application id \"com.google.Chrome\" to tell active tab of front window to return (URL as text) & linefeed & (title as text)"
        )
    );
}

#[test]
fn tab_script_has_no_script_for_apps_that_are_not_browsers_it_can_read() {
    assert_eq!(tab_script("org.mozilla.firefox"), None);
    assert!(!is_browser("com.microsoft.VSCode"));
}

#[test]
fn tab_script_never_puts_an_odd_bundle_id_into_a_script() {
    assert_eq!(tab_script("com.apple.Safari\" to do shell script \"x"), None);
    assert_eq!(tab_script(""), None);
    assert_eq!(tab_script("com.apple.Safari\n"), None);
}

/// CAP-048: every readable browser, by family.
#[test]
fn reads_twelve_browsers() {
    for id in ["com.apple.Safari", "com.apple.SafariTechnologyPreview"] {
        assert!(is_browser(id));
        assert!(tab_script(id).unwrap().contains("tell front document"));
    }
    for id in [
        "com.google.Chrome",
        "com.google.Chrome.beta",
        "com.google.Chrome.canary",
        "org.chromium.Chromium",
        "company.thebrowser.Browser",
        "company.thebrowser.dia",
        "com.brave.Browser",
        "com.microsoft.edgemac",
        "com.vivaldi.Vivaldi",
        "com.operasoftware.Opera",
    ] {
        assert!(is_browser(id));
        assert!(tab_script(id).unwrap().contains("tell active tab of front window"));
    }
}

/// CAP-050
#[test]
fn reads_the_tab_scripts_answer() {
    let tab = parse_tab("\nhttps://example.com/a\nRefund\nfails #482\n\n", "Safari").unwrap();
    assert_eq!(
        tab,
        TabAttachment {
            url: "https://example.com/a".into(),
            title: "Refund fails #482".into(),
            app: "Safari".into()
        }
    );
    assert_eq!(parse_tab("HTTP://EXAMPLE.COM\n", "Arc").unwrap().title, "");
    assert_eq!(parse_tab("file:///etc/hosts\nhosts", "Arc"), None);
    assert_eq!(parse_tab("about:blank\n", "Arc"), None);
    assert_eq!(parse_tab("", "Arc"), None);
    assert!(is_web_url("Https://x") && !is_web_url("https:/x") && !is_web_url("ftp://x"));
}

fn picture(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, 7, 255])
    }))
}

fn thumb_size(thumb: &str) -> (u32, u32) {
    let data = thumb.strip_prefix("data:image/png;base64,").expect("a PNG data URL");
    let image = decode(&nodus_platform::secrets::decode_base64(data)).expect("a PNG");
    (image.width(), image.height())
}

/// CAP-053
#[test]
fn thumbnails_are_at_most_360_wide() {
    assert_eq!(thumb_size(&thumb_of(&picture(720, 101))), (360, 51));
    assert_eq!(thumb_size(&thumb_of(&picture(361, 1))), (360, 1));
    assert_eq!(thumb_size(&thumb_of(&picture(360, 500))), (360, 500));
    assert_eq!(thumb_size(&thumb_of(&picture(20, 10))), (20, 10));
}

#[test]
fn decodes_only_real_images() {
    assert!(decode(b"not an image").is_none());
    assert!(decode(&png_of(&picture(3, 2))).is_some());
}

/// A clipboard with what the test puts on it.
#[derive(Default)]
struct FakeClipboard {
    text: Option<String>,
    image: Mutex<Option<Vec<u8>>>,
}

impl Clipboard for FakeClipboard {
    fn text(&self) -> Option<String> {
        self.text.clone()
    }

    fn image(&self) -> Option<Vec<u8>> {
        self.image.lock().unwrap().clone()
    }
}

fn clipboard(text: Option<&str>, image: Option<Vec<u8>>) -> FakeClipboard {
    FakeClipboard {
        text: text.map(str::to_owned),
        image: Mutex::new(image),
    }
}

/// CAP-052: text first, trimmed and cut; else an image; else nothing.
#[test]
fn offers_the_clipboard() {
    let png = png_of(&picture(400, 200));
    assert_eq!(
        read_clipboard(&clipboard(Some("  copied \n"), Some(png.clone()))),
        Some(ClipboardOffer::Text { text: "copied".into() })
    );
    let long = "é".repeat(20_005);
    let Some(ClipboardOffer::Text { text }) = read_clipboard(&clipboard(Some(&long), None)) else {
        panic!("text")
    };
    assert_eq!(js::utf16_len(&text), 20_000);
    let Some(ClipboardOffer::Image { thumb }) = read_clipboard(&clipboard(Some(" \n "), Some(png))) else {
        panic!("an image")
    };
    assert_eq!(thumb_size(&thumb), (360, 180));
    assert_eq!(read_clipboard(&clipboard(None, Some(b"junk".to_vec()))), None);
    assert_eq!(read_clipboard(&clipboard(None, None)), None);
}

/// CAP-054, CAP-158: an image taken from the clipboard is saved as a PNG in captures/.
#[test]
fn takes_the_clipboard() {
    let user_data = tempfile::tempdir().unwrap();
    assert_eq!(
        take_clipboard(&clipboard(Some(" a "), None), user_data.path()),
        Some(CaptureAttachment::Text(TextAttachment { text: "a".into() }))
    );
    assert_eq!(take_clipboard(&clipboard(None, None), user_data.path()), None);
    let tiff = {
        let mut out = Cursor::new(Vec::new());
        picture(30, 20).write_to(&mut out, ImageFormat::Tiff).unwrap();
        out.into_inner()
    };
    let Some(CaptureAttachment::Image(image)) = take_clipboard(&clipboard(None, Some(tiff)), user_data.path()) else {
        panic!("an image")
    };
    assert_eq!(image.copied, Some(true));
    assert_eq!(thumb_size(&image.thumb), (30, 20));
    let path = Path::new(&image.path);
    assert_eq!(path.parent().unwrap(), user_data.path().join("captures"));
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(name.ends_with(".png") && uuid::Uuid::parse_str(&name[..name.len() - 4]).is_ok());
    assert_eq!(image::guess_format(&fs::read(path).unwrap()).unwrap(), ImageFormat::Png);
}

#[test]
fn reads_a_windows_clipboard_bitmap() {
    // 2 × 1, 24 bits, rows from the bottom padded to 4 bytes: blue, then red (BGR).
    let mut dib = Vec::new();
    for value in [40u32, 2, 1] {
        dib.extend_from_slice(&value.to_le_bytes());
    }
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&24u16.to_le_bytes());
    dib.extend_from_slice(&[0; 24]);
    dib.extend_from_slice(&[255, 0, 0, 0, 0, 255, 0, 0]);
    let bmp = dib_to_bmp(&dib).unwrap();
    assert_eq!(&bmp[..2], b"BM");
    assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
    let image = decode(&bmp).unwrap().to_rgba8();
    assert_eq!(image.get_pixel(0, 0), &Rgba([0, 0, 255, 255]));
    assert_eq!(image.get_pixel(1, 0), &Rgba([255, 0, 0, 255]));
    assert_eq!(dib_to_bmp(&[1, 2, 3]), None);
}

#[test]
fn screenshots_answer_as_0_9_4_does() {
    let image = Screenshot::Image(ImageAttachment {
        path: "/x.png".into(),
        thumb: "data:".into(),
        copied: None,
    });
    let json = |s: &Screenshot| nodus_domain::json::to_string(s).unwrap();
    assert_eq!(json(&image), r#"{"kind":"image","path":"/x.png","thumb":"data:"}"#);
    assert_eq!(json(&Screenshot::NeedsPermission), r#""needs-permission""#);
    assert_eq!(json(&Screenshot::None), "null");
}

/// CAP-057
#[test]
fn reads_the_shot_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shot.png");
    assert_eq!(read_shot(&file, false), Screenshot::None);
    fs::write(&file, b"garbage").unwrap();
    assert_eq!(read_shot(&file, false), Screenshot::None);
    fs::write(&file, png_of(&picture(500, 100))).unwrap();
    let Screenshot::Image(image) = read_shot(&file, false) else {
        panic!("an image")
    };
    assert_eq!((image.path.as_str(), image.copied), (file.to_str().unwrap(), None));
    assert_eq!(thumb_size(&image.thumb), (360, 72));
}

// A shell script stands in for screencapture.
#[cfg(unix)]
mod screencapture {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A screencapture that logs its arguments and runs `body` (`$4` is the file).
    fn stand_in(dir: &Path, body: &str) -> String {
        let script = dir.join("screencapture");
        let text = format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> '{}/argv.log'; done\n{body}\n",
            dir.display()
        );
        fs::write(&script, text).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    /// CAP-055: `-i -x -o <file>`; the file it writes is the shot.
    #[tokio::test]
    async fn takes_a_shot_with_screencapture() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        fs::write(&source, png_of(&picture(40, 30))).unwrap();
        let tool = stand_in(dir.path(), &format!("cp '{}' \"$4\"", source.display()));
        let file = new_shot_path(dir.path());
        let asked = std::cell::Cell::new(false);
        let blocked = shoot_interactively(&tool, &file, || {
            asked.set(true);
            false
        })
        .await;
        assert_eq!(blocked, None);
        // The permission is read only after a shot that didn't come.
        assert!(!asked.get());
        let argv = fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert_eq!(argv, format!("-i\n-x\n-o\n{}\n", file.display()));
        assert!(matches!(read_shot(&file, false), Screenshot::Image(_)));
    }

    /// CAP-055: no file and no permission is `needs-permission`; no file
    /// with it is a cancel; the tool failing changes neither.
    #[tokio::test]
    async fn tells_a_cancel_from_a_missing_permission() {
        let dir = tempfile::tempdir().unwrap();
        let tool = stand_in(dir.path(), "exit 1");
        let file = dir.path().join("none.png");
        assert_eq!(
            shoot_interactively(&tool, &file, || false).await,
            Some(Screenshot::NeedsPermission)
        );
        assert_eq!(shoot_interactively(&tool, &file, || true).await, None);
        assert_eq!(read_shot(&file, false), Screenshot::None);
        let missing = dir.path().join("no-such-tool").to_string_lossy().into_owned();
        assert_eq!(
            shoot_interactively(&missing, &file, || false).await,
            Some(Screenshot::NeedsPermission)
        );
    }

    /// A tool named `name` that logs each call's arguments on a line
    /// (separated by `|`) and runs `body`.
    fn tool(dir: &Path, name: &str, body: &str) -> String {
        let script = dir.join(name);
        let text = format!(
            "#!/bin/sh\nlog='{}/{name}.log'\nfirst=1\nfor a in \"$@\"; do if [ $first = 1 ]; then first=0; else printf '|' >> \"$log\"; fi; printf '%s' \"$a\" >> \"$log\"; done\necho >> \"$log\"\n{body}\n",
            dir.display()
        );
        fs::write(&script, text).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    /// macOS checks a script it hasn't run before, which can take longer
    /// than lsappinfo's own 1 s on a busy machine.
    const SLOW: Duration = Duration::from_secs(30);

    /// CAP-045: `lsappinfo front`, then `lsappinfo info` on its ASN.
    #[tokio::test]
    async fn reads_the_app_in_front_with_lsappinfo() {
        let dir = tempfile::tempdir().unwrap();
        let lsappinfo = tool(
            dir.path(),
            "lsappinfo",
            "if [ \"$1\" = front ]; then echo ' ASN:0x0-0x1a01a: '; else printf '\"Safari\" ASN:0x0-0x1a01a: (in front)\\n    bundleID=\"com.apple.Safari\"\\n'; fi",
        );
        assert_eq!(
            front_app_with(&lsappinfo, SLOW).await,
            Some(FrontApp {
                name: "Safari".into(),
                bundle_id: "com.apple.Safari".into()
            })
        );
        let log = fs::read_to_string(dir.path().join("lsappinfo.log")).unwrap();
        assert_eq!(log, "front\ninfo|-only|bundleid|-only|name|ASN:0x0-0x1a01a:\n");
        // No ASN, a failure, or no tool at all: no app.
        let empty = tool(dir.path(), "empty", "echo '  '");
        assert_eq!(front_app_with(&empty, SLOW).await, None);
        let failing = tool(dir.path(), "failing", "echo 'ASN:0x1:'; exit 1");
        assert_eq!(front_app_with(&failing, SLOW).await, None);
        assert_eq!(
            front_app_with(&dir.path().join("none").to_string_lossy(), SLOW).await,
            None
        );
    }

    /// CAP-050: `osascript -e <script>`, the URL then the title.
    #[tokio::test]
    async fn reads_the_tab_with_osascript() {
        let dir = tempfile::tempdir().unwrap();
        let osascript = tool(dir.path(), "osascript", "printf 'https://example.com/a\\nA title\\n'");
        let arc = FrontApp {
            name: "Arc".into(),
            bundle_id: "company.thebrowser.Browser".into(),
        };
        assert_eq!(
            browser_tab_with(&osascript, &arc).await,
            Some(TabAttachment {
                url: "https://example.com/a".into(),
                title: "A title".into(),
                app: "Arc".into()
            })
        );
        let log = fs::read_to_string(dir.path().join("osascript.log")).unwrap();
        assert_eq!(log, format!("-e|{}\n", tab_script(&arc.bundle_id).unwrap()));
        // Not a readable browser: nothing is run.
        let firefox = FrontApp {
            name: "Firefox".into(),
            bundle_id: "org.mozilla.firefox".into(),
        };
        assert_eq!(browser_tab_with(&osascript, &firefox).await, None);
        assert_eq!(
            fs::read_to_string(dir.path().join("osascript.log"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        // Refused (macOS's Automation prompt answered no): no tab.
        let refused = tool(dir.path(), "refused", "echo 'Not authorized' >&2; exit 1");
        assert_eq!(browser_tab_with(&refused, &arc).await, None);
    }
}

/// CAP-061, CAP-171
#[test]
fn describes_files_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("checkout.log");
    fs::write(&file, b"twelve bytes").unwrap();
    let folder = dir.path().join("logs");
    fs::create_dir(&folder).unwrap();
    let paths = [
        file.to_string_lossy().into_owned(),
        format!("{}/", folder.display()),
        dir.path().join("gone").to_string_lossy().into_owned(),
    ];
    assert_eq!(
        describe_files(&paths),
        vec![
            CaptureAttachment::File(FileAttachment {
                path: paths[0].clone(),
                name: "checkout.log".into(),
                size: 12
            }),
            CaptureAttachment::File(FileAttachment {
                path: paths[1].clone(),
                name: "logs".into(),
                size: 0
            }),
        ]
    );
    let many: Vec<String> = (0..25).map(|_| paths[0].clone()).collect();
    assert_eq!(describe_files(&many).len(), 20);
    // Dropped paths must be absolute.
    let dropped = ["checkout.log".to_owned(), paths[0].clone()];
    assert_eq!(describe_dropped(&dropped).len(), 1);
}

#[test]
fn paths_read_as_node_reads_them() {
    assert_eq!(basename("/a/b/c.txt"), "c.txt");
    assert_eq!(basename("/a/b//"), "b");
    assert_eq!(basename("/"), "");
    if cfg!(windows) {
        assert!(is_absolute("C:\\x") && is_absolute("\\x") && is_absolute("/x") && !is_absolute("C:x"));
        assert_eq!(basename("C:\\a\\b.txt"), "b.txt");
    } else {
        assert!(is_absolute("/x") && !is_absolute("x/y") && !is_absolute("C:\\x"));
        assert_eq!(basename("a\\b"), "a\\b");
    }
}

fn age(path: &Path, days: f64) {
    let when = SystemTime::now() - Duration::from_secs_f64(days * 86_400.0);
    let mut options = fs::File::options();
    options.read(true);
    // Windows sets a time only through a handle with FILE_WRITE_ATTRIBUTES,
    // and opens a folder only with FILE_FLAG_BACKUP_SEMANTICS.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.access_mode(0x0100).custom_flags(0x0200_0000);
    }
    options.open(path).unwrap().set_modified(when).unwrap();
}

/// CAP-063, CAP-173: by mtime, in name order, stopping at an old folder.
#[test]
fn clears_away_old_captures() {
    let user_data = tempfile::tempdir().unwrap();
    let dir = shots_dir(user_data.path());
    for name in ["a-old.png", "b-new.png", "d-old.png"] {
        fs::write(dir.join(name), b"x").unwrap();
    }
    age(&dir.join("a-old.png"), 31.0);
    age(&dir.join("b-new.png"), 29.0);
    age(&dir.join("d-old.png"), 31.0);
    fs::create_dir(dir.join("c-folder")).unwrap();
    age(&dir.join("c-folder"), 40.0);
    prune_shots(user_data.path(), now_ms());
    let mut left: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    // The old folder stopped the sweep before d-old.png.
    assert_eq!(left, ["b-new.png", "c-folder", "d-old.png"]);

    // A missing folder is made, and an empty one is fine.
    let fresh = tempfile::tempdir().unwrap();
    prune_shots(fresh.path(), now_ms());
    assert!(fresh.path().join("captures").is_dir());
}
