//! What floating capture brings from the user's screen: the app in front and
//! its browser tab, a screenshot, what's on the clipboard, and files. A port
//! of `src/main/screenContext.ts`, and of `describeFiles` in capture.ts.
//!
//! Every program it runs is macOS's own (`lsappinfo`, `osascript`,
//! `screencapture`), started through `nodus_platform::process` as Node's
//! `execFile` would: the app's environment, a timeout, 1 MiB of output.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageFormat, imageops::FilterType};
use nodus_domain::js;
use nodus_domain::{
    CaptureAttachment, ClipboardOffer, FileAttachment, ImageAttachment, MAX_COPIED_CHARS, TabAttachment,
};
use nodus_platform::process::{Command, NODE_MAX_BUFFER};
use nodus_platform::secrets::encode_base64;
use serde::{Serialize, Serializer};

/// macOS's own screenshot tool (CAP-055).
pub const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
/// Stands in for [`SCREENCAPTURE`] when set and not empty (tests).
pub const SCREENCAPTURE_ENV: &str = "NODUS_SCREENCAPTURE";
/// How long the user has to pick what to shoot.
pub const SCREENSHOT_TIMEOUT: Duration = Duration::from_millis(5 * 60_000);
/// `lsappinfo` answers in a few milliseconds; past this it's no answer (CAP-045).
pub const LSAPPINFO_TIMEOUT: Duration = Duration::from_millis(1000);
/// Long enough for the user to answer macOS asking the first time (CAP-050).
pub const TAB_TIMEOUT: Duration = Duration::from_millis(60_000);
/// Screenshots and copied images older than this are cleared away at launch (CAP-063).
pub const KEEP_MS: f64 = 30.0 * 24.0 * 60.0 * 60.0 * 1000.0;
/// Thumbnails are at most this wide (CAP-053).
pub const THUMB_W: u32 = 360;
/// At most this many files are described at once (CAP-061).
pub const MAX_FILES: usize = 20;

/// The app in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontApp {
    pub name: String,
    pub bundle_id: String,
}

/// `execFile(file, args, { timeout })`: its stdout, or nothing when it
/// couldn't start, failed, timed out or printed too much.
async fn run(file: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let output = Command::new(file)
        .args(args)
        .timeout(timeout)
        .max_buffer(NODE_MAX_BUFFER)
        .exec()
        .await
        .ok()?;
    Some(output.stdout_text())
}

/// Reads `lsappinfo info` output: the app's name on the first line, then
/// `bundleID="…"` (CAP-046).
pub fn parse_app_info(out: &str) -> Option<FrontApp> {
    let bundle_id = bundle_id_of(out)?;
    let name = app_name_of(out).unwrap_or_else(|| bundle_id.rsplit('.').next().unwrap_or_default().to_owned());
    Some(FrontApp { name, bundle_id })
}

/// `/bundleID="([^"]+)"/`: the first quoted id that isn't empty.
fn bundle_id_of(out: &str) -> Option<String> {
    const KEY: &str = "bundleID=\"";
    let mut from = 0;
    while let Some(found) = out[from..].find(KEY) {
        let start = from + found + KEY.len();
        if let Some(len) = out[start..].find('"').filter(|len| *len > 0) {
            return Some(out[start..start + len].to_owned());
        }
        from = start;
    }
    None
}

/// `/^\s*"([^"]+)"\s+ASN:/m`: a quoted name at the start of a line, followed by `ASN:`.
fn app_name_of(out: &str) -> Option<String> {
    let line_starts = std::iter::once(0).chain(
        out.char_indices()
            .filter(|(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            .map(|(at, c)| at + c.len_utf8()),
    );
    line_starts.into_iter().find_map(|start| {
        let rest = js::trim_start(&out[start..]);
        let rest = rest.strip_prefix('"')?;
        let len = rest.find('"').filter(|len| *len > 0)?;
        let after = &rest[len + 1..];
        let spaced = js::trim_start(after);
        (spaced.len() < after.len() && spaced.starts_with("ASN:")).then(|| rest[..len].to_owned())
    })
}

/// The app in front, on a Mac (CAP-045). `lsappinfo` needs no permission and
/// answers in a few milliseconds. Elsewhere there's none.
pub async fn front_app() -> Option<FrontApp> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let asn = run("/usr/bin/lsappinfo", &["front"], LSAPPINFO_TIMEOUT).await?;
    let asn = js::trim(&asn);
    if asn.is_empty() {
        return None;
    }
    let info = run(
        "/usr/bin/lsappinfo",
        &["info", "-only", "bundleid", "-only", "name", asn],
        LSAPPINFO_TIMEOUT,
    )
    .await?;
    parse_app_info(&info)
}

/// Browsers whose front document AppleScript can read (CAP-048).
const SAFARI: [&str; 2] = ["com.apple.Safari", "com.apple.SafariTechnologyPreview"];
/// Chromium browsers whose active tab AppleScript can read. Firefox has no way to.
const CHROMIUM: [&str; 10] = [
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
];

/// Whether the app is a browser whose tab can be read (CAP-048).
pub fn is_browser(bundle_id: &str) -> bool {
    SAFARI.contains(&bundle_id) || CHROMIUM.contains(&bundle_id)
}

/// The script that reads a browser's front tab: its URL, then its title, a
/// line each (CAP-049).
pub fn tab_script(bundle_id: &str) -> Option<String> {
    // `/^[\w.-]+$/`: bundle ids are dotted names; anything else never reaches a script.
    let plain = !bundle_id.is_empty()
        && bundle_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if !plain {
        return None;
    }
    if SAFARI.contains(&bundle_id) {
        return Some(format!(
            "tell application id \"{bundle_id}\" to tell front document to return (URL as text) & linefeed & (name as text)"
        ));
    }
    if CHROMIUM.contains(&bundle_id) {
        return Some(format!(
            "tell application id \"{bundle_id}\" to tell active tab of front window to return (URL as text) & linefeed & (title as text)"
        ));
    }
    None
}

/// `/^https?:\/\//i`.
pub fn is_web_url(url: &str) -> bool {
    let lower = |n: usize| url.get(..n).map(str::to_ascii_lowercase);
    lower(7).as_deref() == Some("http://") || lower(8).as_deref() == Some("https://")
}

/// What the tab script printed, as the tab (CAP-050): the first line is the
/// URL, which must be a web page's; the rest is the title.
pub fn parse_tab(out: &str, app: &str) -> Option<TabAttachment> {
    let mut lines = js::trim(out).split('\n');
    let url = lines.next().unwrap_or_default();
    if !is_web_url(url) {
        return None;
    }
    let title = lines.collect::<Vec<_>>().join(" ");
    Some(TabAttachment {
        url: url.to_owned(),
        title: js::trim(&title).to_owned(),
        app: app.to_owned(),
    })
}

/// The front tab of a browser in front (CAP-050). macOS asks once, for each
/// browser, whether Nodus may read it; until it may, there's no tab.
pub async fn browser_tab(front: &FrontApp) -> Option<TabAttachment> {
    let script = tab_script(&front.bundle_id)?;
    let out = run("/usr/bin/osascript", &["-e", &script], TAB_TIMEOUT).await?;
    parse_tab(&out, &front.name)
}

/// Where screenshots and copied images are kept, made if it isn't there
/// (CAP-158).
pub fn shots_dir(user_data: &Path) -> PathBuf {
    let dir = user_data.join("captures");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// `<captures>/<random UUID v4>.png` (CAP-158).
pub fn new_shot_path(user_data: &Path) -> PathBuf {
    shots_dir(user_data).join(format!("{}.png", uuid::Uuid::new_v4()))
}

/// An image as a PNG.
pub fn png_of(image: &DynamicImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let _ = DynamicImage::ImageRgba8(image.to_rgba8()).write_to(&mut out, ImageFormat::Png);
    out.into_inner()
}

/// A small preview: no wider than 360 px, aspect kept, as a PNG data URL (CAP-053).
pub fn thumb_of(image: &DynamicImage) -> String {
    let (w, h) = (image.width(), image.height());
    let small;
    let image = if w > THUMB_W {
        // Electron scales the height with the width, rounded.
        let height = js::round(f64::from(h) * f64::from(THUMB_W) / f64::from(w)).max(1.0) as u32;
        small = image.resize_exact(THUMB_W, height, FilterType::CatmullRom);
        &small
    } else {
        image
    };
    format!("data:image/png;base64,{}", encode_base64(&png_of(image)))
}

/// An image file or image bytes, decoded; nothing when it can't be read or
/// has no pixels (`nativeImage.isEmpty()`).
pub fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    image::load_from_memory(bytes)
        .ok()
        .filter(|image| image.width() > 0 && image.height() > 0)
}

/// What taking a screenshot came to (`capture:screenshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screenshot {
    /// The shot, saved in `captures/`.
    Image(ImageAttachment),
    /// macOS kept the shot from Nodus: no Screen Recording permission.
    NeedsPermission,
    /// The user cancelled, or nothing came.
    None,
}

impl Serialize for Screenshot {
    /// As 0.9.4 answers: the image attachment, `'needs-permission'` or `null`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Image(image) => CaptureAttachment::Image(image.clone()).serialize(serializer),
            Self::NeedsPermission => serializer.serialize_str("needs-permission"),
            Self::None => serializer.serialize_none(),
        }
    }
}

/// The screenshot tool: [`SCREENCAPTURE`], or `NODUS_SCREENCAPTURE` when it's
/// set and not empty (CAP-055).
pub fn screencapture_path() -> String {
    std::env::var(SCREENCAPTURE_ENV)
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| SCREENCAPTURE.into())
}

/// A screenshot the user takes: on a Mac, macOS's own crosshair (Space for a
/// window), and off a Mac the whole screen the pointer is on. The panel is
/// hidden first, so it isn't in the shot (CAP-055…CAP-057).
///
/// On a Mac, screencapture runs on Nodus's Screen Recording permission. macOS
/// doesn't ask for it: without it the crosshair still comes up, and the shot
/// fails once the user has picked. That comes back as `NeedsPermission`.
pub async fn take_screenshot(user_data: &Path) -> Screenshot {
    let file = new_shot_path(user_data);
    #[cfg(windows)]
    windows::shoot_screen_under_pointer(&file);
    #[cfg(not(windows))]
    {
        let granted = nodus_platform::permissions::screen_capture_granted;
        if let Some(blocked) = shoot_interactively(&screencapture_path(), &file, granted).await {
            return blocked;
        }
    }
    read_shot(&file, false)
}

/// macOS's half of [`take_screenshot`]: runs the tool on `file` (`-i` lets
/// the user choose, `-x` keeps it quiet, `-o` leaves out a window's shadow),
/// and answers `NeedsPermission` when no file came and Screen Recording isn't
/// granted. The permission is read only after a shot that didn't come, as
/// Nodus's own reading can lag behind a permission just given.
pub async fn shoot_interactively(tool: &str, file: &Path, granted: impl FnOnce() -> bool) -> Option<Screenshot> {
    let path = file.to_string_lossy();
    // Its errors don't matter: whether a file came does.
    let _ = run(tool, &["-i", "-x", "-o", &path], SCREENSHOT_TIMEOUT).await;
    (!file.exists() && !granted()).then_some(Screenshot::NeedsPermission)
}

/// The shot in `file`, if one came and it's an image (CAP-057).
pub fn read_shot(file: &Path, copied: bool) -> Screenshot {
    let Some(image) = fs::read(file).ok().and_then(|bytes| decode(&bytes)) else {
        return Screenshot::None;
    };
    Screenshot::Image(ImageAttachment {
        path: file.to_string_lossy().into_owned(),
        thumb: thumb_of(&image),
        copied: copied.then_some(true),
    })
}

/// The system clipboard as capture reads it.
pub trait Clipboard: Send + Sync {
    /// Plain text on it, if any (Electron's `clipboard.readText()`).
    fn text(&self) -> Option<String>;
    /// The encoded bytes of the first image on it, if any (PNG, TIFF, BMP…).
    fn image(&self) -> Option<Vec<u8>>;
}

/// The user's clipboard: NSPasteboard on a Mac, the Windows clipboard on Windows.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn text(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        return mac::clipboard_text();
        #[cfg(windows)]
        return windows::clipboard_text();
        #[cfg(not(any(target_os = "macos", windows)))]
        None
    }

    fn image(&self) -> Option<Vec<u8>> {
        #[cfg(target_os = "macos")]
        return mac::clipboard_image();
        #[cfg(windows)]
        return windows::clipboard_image();
        #[cfg(not(any(target_os = "macos", windows)))]
        None
    }
}

/// A device-independent bitmap from the Windows clipboard (CF_DIB: the
/// header, its colour masks and table, then the pixels) as a BMP file: the
/// same bytes after a 14-byte file header saying where the pixels start.
pub fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| {
        dib.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let header = u32_at(0)?;
    let bit_count = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]);
    let compression = u32_at(16)?;
    let used = u32_at(32)?;
    // BI_BITFIELDS and BI_ALPHABITFIELDS put their masks after a plain 40-byte header.
    let masks = match (header, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let colors = if used > 0 {
        used
    } else if bit_count <= 8 {
        1 << bit_count
    } else {
        0
    };
    let offset = 14 + header + masks + colors * 4;
    let size = u32::try_from(14 + dib.len()).ok()?;
    if offset > size {
        return None;
    }
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// The clipboard's text, trimmed and cut to 20 000 UTF-16 units; empty when
/// there's none.
fn clipboard_text(clipboard: &dyn Clipboard) -> String {
    clipboard
        .text()
        .map(|text| js::slice_utf16(js::trim(&text), 0, MAX_COPIED_CHARS))
        .unwrap_or_default()
}

/// What's on the clipboard, to offer: text, or an image as a small preview
/// (CAP-052). Text wins: a file copied in Finder also puts its icon there.
pub fn read_clipboard(clipboard: &dyn Clipboard) -> Option<ClipboardOffer> {
    let text = clipboard_text(clipboard);
    if !text.is_empty() {
        return Some(ClipboardOffer::Text { text });
    }
    let image = clipboard.image().and_then(|bytes| decode(&bytes))?;
    Some(ClipboardOffer::Image {
        thumb: thumb_of(&image),
    })
}

/// The clipboard, attached: text as it is, an image saved as a file Claude
/// can read (CAP-054). It's read afresh, so it can differ from the offer.
pub fn take_clipboard(clipboard: &dyn Clipboard, user_data: &Path) -> Option<CaptureAttachment> {
    let text = clipboard_text(clipboard);
    if !text.is_empty() {
        return Some(CaptureAttachment::Text(nodus_domain::TextAttachment { text }));
    }
    let image = clipboard.image().and_then(|bytes| decode(&bytes))?;
    let file = new_shot_path(user_data);
    // Like writeFileSync: a write that fails fails the call.
    fs::write(&file, png_of(&image)).ok()?;
    Some(CaptureAttachment::Image(ImageAttachment {
        path: file.to_string_lossy().into_owned(),
        thumb: thumb_of(&image),
        copied: Some(true),
    }))
}

/// `path.isAbsolute` as Node has it on this OS: on Windows a path that
/// starts with a slash or backslash counts, and so does `C:\` or `C:/`.
pub fn is_absolute(path: &str) -> bool {
    if cfg!(windows) {
        let bytes = path.as_bytes();
        let slash = |b: u8| b == b'/' || b == b'\\';
        bytes.first().is_some_and(|b| slash(*b))
            || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && slash(bytes[2]))
    } else {
        path.starts_with('/')
    }
}

/// `path.basename`: the last part, without trailing separators.
pub fn basename(path: &str) -> String {
    let separator = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = path.trim_end_matches(separator);
    // Windows: `C:` alone has no name after it.
    let trimmed = if cfg!(windows) && trimmed.len() == 2 && trimmed.ends_with(':') {
        ""
    } else {
        trimmed
    };
    trimmed.rsplit(separator).next().unwrap_or_default().to_owned()
}

/// Files dropped on the panel or picked, as tiles: at most the first 20; a
/// file or a folder (size 0) each; anything else, or gone, left out
/// (CAP-061, CAP-171).
pub fn describe_files(paths: &[String]) -> Vec<CaptureAttachment> {
    paths
        .iter()
        .take(MAX_FILES)
        .filter_map(|path| {
            let meta = fs::metadata(path).ok()?;
            (meta.is_file() || meta.is_dir()).then(|| {
                CaptureAttachment::File(FileAttachment {
                    path: path.clone(),
                    name: basename(path),
                    size: if meta.is_file() { meta.len() } else { 0 },
                })
            })
        })
        .collect()
}

/// `capture:files(paths)` for dropped files: only absolute paths are taken
/// (CAP-060).
pub fn describe_dropped(paths: &[String]) -> Vec<CaptureAttachment> {
    let absolute: Vec<String> = paths.iter().filter(|p| is_absolute(p)).cloned().collect();
    describe_files(&absolute)
}

/// Clears away screenshots and copied images older than 30 days, by when
/// they were last changed (CAP-063). The first thing that fails stops it, as
/// 0.9.4's does: a folder that old can't be removed, so the sweep ends there
/// (CAP-173). Entries go in name order, as Node lists them.
pub fn prune_shots(user_data: &Path, now: f64) {
    let dir = shots_dir(user_data);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut names: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    names.sort();
    for name in names {
        let file = dir.join(name);
        let Ok(meta) = fs::metadata(&file) else {
            return;
        };
        if now - mtime_ms(&meta) <= KEEP_MS {
            continue;
        }
        // rmSync without `recursive` refuses a folder.
        let is_dir = fs::symlink_metadata(&file).is_ok_and(|m| m.is_dir());
        if is_dir {
            return;
        }
        match fs::remove_file(&file) {
            Ok(()) => {}
            // `force: true` takes a file already gone as removed.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return,
        }
    }
}

/// `stat.mtimeMs`.
fn mtime_ms(meta: &fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// `Date.now()`.
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_millis() as f64)
}

#[cfg(test)]
mod tests;
