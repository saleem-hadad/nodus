//! Help › Report a Problem… (docs/specs/13-feedback.md): a dialog that turns
//! what the user writes, and the screenshots they add, into an issue on the
//! Nodus repository through the feedback server. Three steps: Write, Review
//! (exactly what will be public) and Done. When the server can't be reached,
//! GitHub's new-issue page takes over in the browser, filled in.

mod render;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::component::input::{InputEvent, InputState, TextareaState};
use gpui::{
    AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, FocusHandle, Image, ImageFormat, Subscription,
    Window,
};
use nodus_core::{AreaShot, Nodus};
use nodus_feedback::images::kind_of;
use nodus_feedback::{ClientInfo, MAX_SHOTS, Posted, PrepareError, Report, Scrub, SendError};

use crate::components::text_field::new_input;
use crate::platform::Platform;
use crate::state::AppState;

/// A dropped file larger than this isn't read (RPT-012 shrinks what's
/// read, but not without end).
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;

/// A screenshot on the report: the bytes that are sent, and the picture
/// drawn for them.
#[derive(Clone)]
struct Shot {
    bytes: Arc<Vec<u8>>,
    image: Arc<Image>,
}

enum Step {
    Write,
    Review,
    /// RPT-036: the issue it became.
    Done(Posted),
    /// RPT-043: the browser has the issue, filled in.
    Browser {
        clipboard: bool,
        folder: bool,
    },
}

/// Something to say about the screenshots (RPT-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Full,
    Unsupported,
    TooLarge,
    NeedsPermission,
    NoWindowShot,
}

/// Why the last try to post didn't (RPT-034, RPT-035).
struct Failure {
    title: &'static str,
    message: String,
    /// The browser can still take it.
    browser: bool,
}

pub struct ReportDialog {
    state: Entity<AppState>,
    nodus: Nodus,
    /// What had focus before, to give back on close.
    pub previous_focus: Option<FocusHandle>,
    panel: FocusHandle,
    step: Step,
    message: Entity<TextareaState>,
    username: Entity<InputState>,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    shots: Vec<Shot>,
    notice: Option<Notice>,
    username_invalid: bool,
    /// What Review was last drafted from, so going back and forth keeps
    /// edits made there (RPT-025).
    drafted: Option<(String, Option<String>)>,
    reporter: Option<String>,
    sending: bool,
    failure: Option<Failure>,
    /// RPT-007, RPT-008: stepped aside while a screenshot is taken.
    shooting: bool,
    scrub: Scrub,
    _subscriptions: Vec<Subscription>,
}

impl ReportDialog {
    pub fn new(
        state: Entity<AppState>,
        previous_focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let nodus = state.read(cx).nodus.clone();
        let panel = cx.focus_handle();
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 10)
                .placeholder("What happened? What did you expect instead?")
                .context_menu(false)
        });
        // RPT-006: the username the last report named.
        let remembered = nodus.feedback().username().unwrap_or_default();
        let username = new_input(&remembered, "your-username", false, window, cx);
        let title = new_input("", "Title", false, window, cx);
        let body = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 12).context_menu(false));
        let subscriptions = vec![
            cx.subscribe_in(&message, window, |this, _, _: &InputEvent, _, cx| this.edited(cx)),
            cx.subscribe_in(&username, window, |this, _, _: &InputEvent, _, cx| {
                this.username_invalid = false;
                cx.notify();
            }),
            cx.subscribe_in(&title, window, |this, _, _: &InputEvent, _, cx| this.edited(cx)),
            cx.subscribe_in(&body, window, |this, _, _: &InputEvent, _, cx| this.edited(cx)),
        ];
        // RPT-024: what to take out, read off the UI thread.
        let scrub_nodus = nodus.clone();
        cx.spawn(async move |this, cx| {
            let scrub = scrub_nodus.feedback().scrub().await;
            let _ = this.update(cx, |d, _| d.scrub = scrub);
        })
        .detach();
        // RPT-003: the cursor starts in the description.
        let field = message.clone();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| field.update(cx, |f, cx| f.focus(window, cx)));
        });
        Self {
            state,
            nodus,
            previous_focus,
            panel,
            step: Step::Write,
            message,
            username,
            title,
            body,
            shots: Vec::new(),
            notice: None,
            username_invalid: false,
            drafted: None,
            reporter: None,
            sending: false,
            failure: None,
            shooting: false,
            scrub: Scrub::default(),
            _subscriptions: subscriptions,
        }
    }

    /// Typing clears the last failure, and the buttons follow the text.
    fn edited(&mut self, cx: &mut Context<'_, Self>) {
        if !self.sending {
            self.failure = None;
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.state.update(cx, AppState::close_report);
    }

    fn message_text(&self, cx: &Context<'_, Self>) -> String {
        self.message.read(cx).value().to_string()
    }

    fn can_review(&self, cx: &Context<'_, Self>) -> bool {
        !self.shooting && !self.message_text(cx).trim().is_empty()
    }

    fn can_post(&self, cx: &Context<'_, Self>) -> bool {
        !self.sending && !self.title.read(cx).value().trim().is_empty() && !self.body.read(cx).value().trim().is_empty()
    }

    /// RPT-020…RPT-025: Write → Review. The username must be one GitHub
    /// allows; the title and description are drafted from the description
    /// with personal details out, unless nothing changed since the last
    /// draft, whose edits stay.
    fn review(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.can_review(cx) {
            return;
        }
        let message = self.message_text(cx);
        let raw = self.username.read(cx).value().to_string();
        let Ok(reporter) = nodus_feedback::github_username(&raw) else {
            self.username_invalid = true;
            let field = self.username.clone();
            field.update(cx, |f, cx| f.focus(window, cx));
            cx.notify();
            return;
        };
        let key = (message.clone(), reporter.clone());
        if self.drafted.as_ref() != Some(&key) {
            let clean = self.scrub.apply(&message);
            let title = nodus_feedback::draft_title(&clean);
            let body = nodus_feedback::draft_body(&clean, reporter.as_deref());
            self.title.update(cx, |f, cx| f.set_value(title, window, cx));
            self.body.update(cx, |f, cx| f.set_value(body, window, cx));
            self.drafted = Some(key);
        }
        self.reporter = reporter;
        self.failure = None;
        self.step = Step::Review;
        let field = self.body.clone();
        window.on_next_frame(move |window, cx| field.update(cx, |f, cx| f.focus(window, cx)));
        cx.notify();
    }

    /// RPT-026: back to Write, everything kept.
    fn back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sending {
            return;
        }
        self.step = Step::Write;
        self.failure = None;
        let field = self.message.clone();
        window.on_next_frame(move |window, cx| field.update(cx, |f, cx| f.focus(window, cx)));
        cx.notify();
    }

    /// RPT-030: posts what's on Review.
    fn post(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.can_post(cx) {
            return;
        }
        self.sending = true;
        self.failure = None;
        cx.notify();
        let report = Report {
            title: self.title.read(cx).value().trim().to_owned(),
            body: self.body.read(cx).value().trim().to_owned(),
            reporter: self.reporter.clone(),
            client: client_info(),
            shots: self.shots.iter().map(|s| (*s.bytes).clone()).collect(),
        };
        let nodus = self.nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let sent = nodus.feedback().send(report).await;
            let _ = this.update_in(cx, |d, window, cx| d.sent(sent, window, cx));
        })
        .detach();
    }

    /// RPT-033…RPT-036: what posting came to.
    fn sent(&mut self, sent: Result<Posted, SendError>, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.sending = false;
        match sent {
            Ok(posted) => {
                // RPT-006: the username is kept for next time (or forgotten).
                self.nodus.feedback().remember_username(self.reporter.as_deref());
                self.step = Step::Done(posted);
                window.focus(&self.panel, cx);
            }
            Err(SendError::Unreachable) => {
                self.failure = Some(Failure {
                    title: "Couldn't reach the Nodus feedback server",
                    message:
                        "Check your connection and try again, or open the issue on GitHub yourself. It opens filled in."
                            .to_owned(),
                    browser: true,
                });
            }
            Err(SendError::GitHub(message)) => {
                self.failure = Some(Failure {
                    title: "GitHub didn't take the report",
                    message,
                    browser: true,
                });
            }
            Err(SendError::Refused(message)) => {
                self.failure = Some(Failure {
                    title: "The report wasn't sent",
                    message,
                    browser: false,
                });
            }
        }
        cx.notify();
    }

    /// RPT-041…RPT-043: GitHub's new-issue page with the review's title and
    /// description; a description too long for the link goes on the
    /// clipboard, and the screenshots are saved and shown in their folder,
    /// to drag in.
    fn open_in_browser(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let client = client_info();
        let footer = nodus_feedback::footer(&client.version, &client.os, &client.arch);
        let issue = nodus_feedback::offline_issue(
            self.title.read(cx).value().trim(),
            self.body.read(cx).value().trim(),
            &footer,
        );
        if let Some(text) = &issue.clipboard {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        }
        let shots: Vec<Vec<u8>> = self.shots.iter().map(|s| (*s.bytes).clone()).collect();
        let nodus = self.nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let first: Option<PathBuf> = nodus.feedback().keep_for_browser(shots).await;
            if let Some(first) = &first {
                nodus_platform::shell::show_item_in_folder(first);
            }
            nodus_platform::shell::open_external(&issue.url);
            let _ = this.update(cx, |d, cx| {
                d.step = Step::Browser {
                    clipboard: issue.clipboard.is_some(),
                    folder: first.is_some(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn full(&self) -> bool {
        self.shots.len() >= MAX_SHOTS
    }

    /// RPT-007: the Nodus window as it is under the dialog, which steps
    /// aside for it. Pages on the board stay pictures under a clear cover,
    /// as they are while the dialog is up. No permission is needed: the app
    /// draws the window again itself.
    fn shoot_window(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shooting {
            return;
        }
        if self.full() {
            self.notice = Some(Notice::Full);
            cx.notify();
            return;
        }
        self.shooting = true;
        self.notice = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..40 {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                if cx
                    .update(|_, cx| crate::views::swap::pictures_ready(cx))
                    .unwrap_or(true)
                {
                    break;
                }
            }
            let image = cx.update(|window, cx| {
                for _ in 0..2 {
                    window.run_next_frame_callbacks(cx);
                    window.draw(cx).clear(cx);
                }
                window.render_to_image()
            });
            let png = match image {
                Ok(Ok(image)) => cx
                    .background_spawn(async move { nodus_feedback::png_of_rgba(image) })
                    .await
                    .ok(),
                _ => None,
            };
            let _ = this.update(cx, |d, cx| {
                d.shooting = false;
                match png {
                    Some(bytes) => d.push(bytes),
                    None => d.notice = Some(Notice::NoWindowShot),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// RPT-008: part of the screen, picked with the system's crosshair, the
    /// dialog out of the way meanwhile.
    fn shoot_area(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shooting {
            return;
        }
        if self.full() {
            self.notice = Some(Notice::Full);
            cx.notify();
            return;
        }
        self.shooting = true;
        self.notice = None;
        cx.notify();
        let nodus = self.nodus.clone();
        cx.spawn_in(window, async move |this, cx| {
            let shot = nodus.feedback().shoot_area().await;
            let _ = this.update(cx, |d, cx| {
                d.shooting = false;
                match shot {
                    AreaShot::Image(bytes) => d.add(vec![bytes], cx),
                    AreaShot::NeedsPermission => d.notice = Some(Notice::NeedsPermission),
                    AreaShot::None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// RPT-009
    fn open_screen_settings(&mut self, cx: &mut Context<'_, Self>) {
        self.nodus.feedback().open_screen_settings();
        self.notice = None;
        cx.notify();
    }

    /// RPT-011: ⌘V with an image (or image files) on the clipboard attaches
    /// it; anything else pastes as text. True when it attached.
    fn paste(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut found = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => found.push(image.bytes.clone()),
                ClipboardEntry::ExternalPaths(paths) => {
                    found.extend(paths.paths().iter().filter_map(|p| read_image_file(p)));
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if found.is_empty() {
            return false;
        }
        self.add(found, cx);
        true
    }

    /// RPT-011: image files dropped on the dialog.
    fn drop_files(&mut self, paths: &[PathBuf], cx: &mut Context<'_, Self>) {
        let found: Vec<Vec<u8>> = paths.iter().filter_map(|p| read_image_file(p)).collect();
        if found.is_empty() {
            self.notice = Some(Notice::Unsupported);
            cx.notify();
            return;
        }
        self.add(found, cx);
    }

    /// RPT-010, RPT-012: pictures made ready for GitHub off the UI thread,
    /// as many as there's room for.
    fn add(&mut self, mut raw: Vec<Vec<u8>>, cx: &mut Context<'_, Self>) {
        let room = MAX_SHOTS.saturating_sub(self.shots.len());
        self.notice = None;
        if raw.len() > room {
            raw.truncate(room);
            self.notice = Some(Notice::Full);
        }
        if raw.is_empty() {
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            let ready = cx
                .background_spawn(async move { raw.into_iter().map(nodus_feedback::prepare).collect::<Vec<_>>() })
                .await;
            let _ = this.update(cx, |d, cx| {
                for result in ready {
                    match result {
                        Ok(bytes) if !d.full() => d.push(bytes),
                        Ok(_) => d.notice = Some(Notice::Full),
                        Err(PrepareError::Unsupported) => d.notice = Some(Notice::Unsupported),
                        Err(PrepareError::TooLarge) => d.notice = Some(Notice::TooLarge),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn push(&mut self, bytes: Vec<u8>) {
        let format = match kind_of(&bytes) {
            Some("image/jpeg") => ImageFormat::Jpeg,
            Some("image/gif") => ImageFormat::Gif,
            Some("image/webp") => ImageFormat::Webp,
            _ => ImageFormat::Png,
        };
        let image = Arc::new(Image::from_bytes(format, bytes.clone()));
        self.shots.push(Shot {
            bytes: Arc::new(bytes),
            image,
        });
    }

    fn remove_shot(&mut self, at: usize, cx: &mut Context<'_, Self>) {
        if at < self.shots.len() {
            self.shots.remove(at);
            self.notice = None;
            cx.notify();
        }
    }

    /// RPT-004: ⌘↩ (Ctrl+Enter) does the step's main button.
    fn primary(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        match self.step {
            Step::Write => self.review(window, cx),
            Step::Review => self.post(window, cx),
            Step::Done(_) | Step::Browser { .. } => self.close(cx),
        }
    }
}

/// A file's bytes when it's small enough to read and looks like a picture
/// (by its first bytes, or a name `image` can read).
fn read_image_file(path: &std::path::Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_READ_BYTES {
        return None;
    }
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    ) {
        return None;
    }
    std::fs::read(path).ok()
}

/// RPT-023: which copy of Nodus this is.
fn client_info() -> ClientInfo {
    let system = match Platform::CURRENT {
        Platform::Mac => "macOS",
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
    };
    let version = crate::shell::menus::system_version();
    let os = if version.is_empty() {
        system.to_owned()
    } else {
        format!("{system} {version}")
    };
    ClientInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os,
        arch: crate::shell::menus::node_arch().to_owned(),
    }
}
