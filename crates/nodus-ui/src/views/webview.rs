//! One frame's webview (spec 04, "wry: the frame"): a wry child of the
//! board's window (WKWebView on macOS, WebView2 on Windows) that loads the
//! frame's shell (`shell.rs`) on the views' scheme, serves the page from the
//! core (`Views::serve_page`, VIEW-018…021), keeps to its own view
//! (VIEW-028), and hands what the shell relays to [`Incoming`].

use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{Bounds, Pixels, RenderImage, Rgba, Window};
use nodus_core::Nodus;
use tokio::sync::mpsc::UnboundedSender;
use wry::http::{Request, Response};
use wry::{NewWindowResponse, WebViewBuilder};

use super::shell;

/// A message a frame's shell relayed from its page.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub frame: u64,
    pub body: String,
}

/// What a frame's shell document shows.
struct ShellState {
    view_id: String,
    frame: u64,
    version: u32,
    title: String,
    background: String,
}

/// `#rrggbb` for a solid colour.
pub fn css_color(c: Rgba) -> String {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", ch(c.r), ch(c.g), ch(c.b))
}

/// A frame's webview.
pub struct PageWebview {
    pub frame: u64,
    // Dropped before the container that holds it.
    webview: wry::WebView,
    #[cfg(target_os = "macos")]
    container: super::mac::Container,
    shell: Arc<Mutex<ShellState>>,
    /// The zoom and the page's size in CSS px the shell last scaled it to.
    scale: Cell<(f64, f64, f64)>,
    offscreen: Cell<bool>,
    live: Cell<bool>,
}

impl PageWebview {
    /// Makes the webview for frame `frame` of a view, in `window`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        window: &mut Window,
        frame: u64,
        view_id: &str,
        version: u32,
        title: &str,
        background: Rgba,
        nodus: Nodus,
        tx: UnboundedSender<Incoming>,
    ) -> anyhow::Result<Self> {
        let shell_state = Arc::new(Mutex::new(ShellState {
            view_id: view_id.to_owned(),
            frame,
            version,
            title: title.to_owned(),
            background: css_color(background),
        }));
        let url = shell::shell_url(view_id, frame);
        let serve_state = Arc::clone(&shell_state);
        let pending = Rc::new(Cell::new(true));
        let (nav_id, ipc_id) = (view_id.to_owned(), view_id.to_owned());
        let builder = WebViewBuilder::new()
            .with_custom_protocol(shell::SCHEME.into(), move |_, req| serve(&nodus, &serve_state, &req))
            .with_url(url)
            .with_ipc_handler(move |req: Request<String>| {
                // Only the shell speaks for the page (VIEW-044).
                if shell::is_shell(&req.uri().to_string(), &ipc_id, frame) {
                    let _ = tx.send(Incoming {
                        frame,
                        body: req.into_body(),
                    });
                }
            })
            .with_navigation_handler(move |url| {
                let mut p = pending.get();
                let ok = shell::allow_navigation(&url, &nav_id, frame, &mut p);
                pending.set(p);
                ok
            })
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_accept_first_mouse(true)
            .with_devtools(false)
            .with_back_forward_navigation_gestures(false)
            .with_hotkeys_zoom(false)
            .with_focused(false)
            .with_background_color(rgba8(background));
        #[cfg(windows)]
        let builder = {
            use wry::WebViewBuilderExtWindows as _;
            builder
                .with_https_scheme(true)
                .with_default_context_menus(false)
                .with_browser_accelerator_keys(false)
        };
        let webview = builder.build_as_child(&*window)?;
        #[cfg(target_os = "macos")]
        let container = {
            use wry::WebViewExtMacOS as _;
            let parent = super::mac::content_view(window).ok_or_else(|| anyhow::anyhow!("no content view"))?;
            let container =
                super::mac::Container::new(&parent).ok_or_else(|| anyhow::anyhow!("not on the main thread"))?;
            container.adopt(&webview.webview());
            container
        };
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            super::mac::ignore_window_occlusion(&webview.webview());
            super::mac::no_context_menu(&webview.webview());
        }
        Ok(Self {
            frame,
            webview,
            #[cfg(target_os = "macos")]
            container,
            shell: shell_state,
            scale: Cell::new((1.0, 0.0, 0.0)),
            offscreen: Cell::new(true),
            live: Cell::new(true),
        })
    }

    /// Follows the card: `document` is the body on screen, `clip` what of it
    /// may show (the board's edges), `zoom` the board's zoom: the shell lays
    /// the page out at the body's size in flow units and scales it with a
    /// CSS transform, as 0.9.4's board scaled its iframe. `radius` rounds the
    /// bottom corners.
    pub fn place(&self, document: Bounds<Pixels>, clip: Bounds<Pixels>, zoom: f32, radius: f32) {
        let mut visible = document.intersect(&clip);
        let offscreen = visible.size.width < gpui::px(1.) || visible.size.height < gpui::px(1.);
        self.offscreen.set(offscreen);
        self.set_scale(
            f64::from(zoom),
            f64::from(f32::from(document.size.width)),
            f64::from(f32::from(document.size.height)),
        );
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            // Off screen the container shrinks to nothing rather than hiding,
            // so WebKit still counts the page visible: it keeps running and
            // drawing, and its next snapshot isn't blank (R13).
            if offscreen {
                visible = Bounds::new(document.origin, gpui::size(Pixels::ZERO, Pixels::ZERO));
            }
            self.container
                .set_geometry(&self.webview.webview(), visible, document, radius);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (radius, &mut visible);
            let _ = self.webview.set_visible(!offscreen && self.live.get());
            let _ = self.webview.set_bounds(wry::Rect {
                position: wry::dpi::LogicalPosition::new(
                    f64::from(f32::from(document.origin.x)),
                    f64::from(f32::from(document.origin.y)),
                )
                .into(),
                size: wry::dpi::LogicalSize::new(
                    f64::from(f32::from(document.size.width)),
                    f64::from(f32::from(document.size.height)),
                )
                .into(),
            });
        }
    }

    /// Scales the page in the shell: laid out at the body's size in flow
    /// units (`w`×`h` screen px over the zoom), drawn at the zoom. A CSS
    /// transform keeps WebKit's minimum font size out of it, as it stayed out
    /// of 0.9.4's (page zoom bumped zoomed-out text up to 9px).
    fn set_scale(&self, zoom: f64, w: f64, h: f64) {
        let z = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
        let (cw, ch) = (w / z, h / z);
        let (oz, ow, oh) = self.scale.get();
        if (oz - z).abs() < 1e-4 && (ow - cw).abs() < 0.01 && (oh - ch).abs() < 0.01 {
            return;
        }
        self.scale.set((z, cw, ch));
        let n = |v: f64| nodus_domain::json::number_to_string((v * 1000.0).round() / 1000.0);
        let _ = self.webview.evaluate_script(&format!(
            "window.__nodusScale&&window.__nodusScale({},{},{})",
            n(z),
            n(cw),
            n(ch)
        ));
    }

    /// Off screen: the visible part of the body is empty.
    pub fn offscreen(&self) -> bool {
        self.offscreen.get()
    }

    /// Live (the page shows and takes the pointer) or behind a snapshot.
    pub fn set_live(&self, live: bool) {
        self.live.set(live);
        #[cfg(target_os = "macos")]
        self.container.set_alpha(if live { 1.0 } else { 0.0 });
        #[cfg(not(target_os = "macos"))]
        let _ = self.webview.set_visible(live && !self.offscreen.get());
    }

    /// Shows the board's ink over the live page (`ink.rs`); None hides it.
    /// WebView2 has no overlay yet: there the ink stays under a live page.
    pub fn set_ink(&self, ink: Option<&super::ink::PageInk>) {
        #[cfg(target_os = "macos")]
        self.container.set_ink(ink);
        #[cfg(not(target_os = "macos"))]
        let _ = ink;
    }

    /// The pointer goes to the board instead of the page (VIEW-141).
    pub fn set_passthrough(&self, on: bool) {
        #[cfg(target_os = "macos")]
        self.container.set_passthrough(on);
        #[cfg(not(target_os = "macos"))]
        let _ = on;
    }

    /// The pointer goes to the board this close to the page's sides and
    /// bottom: the resize handles there (VIEW-140).
    pub fn set_edge(&self, edge: f32) {
        #[cfg(target_os = "macos")]
        self.container.set_edge(f64::from(edge));
        #[cfg(not(target_os = "macos"))]
        let _ = edge;
    }

    /// Posts a message to the page (`frame.contentWindow.postMessage(m, '*')`).
    pub fn deliver(&self, message: &serde_json::Value) {
        let json = nodus_domain::json::to_string(message).unwrap_or_else(|_| "null".into());
        let _ = self
            .webview
            .evaluate_script(&format!("window.__nodusDeliver&&window.__nodusDeliver({json})"));
    }

    /// A new version reloads the page (VIEW-027); the title is the frame's
    /// accessible name; the background follows the theme.
    pub fn update(&self, version: u32, title: &str, background: Rgba) {
        let background = css_color(background);
        let (src, title_arg, bg_arg) = {
            let Ok(mut s) = self.shell.lock() else { return };
            let src = (s.version != version).then(|| shell::page_url(&s.view_id, version));
            let title_changed = s.title != title;
            let bg_changed = s.background != background;
            s.version = version;
            s.title = title.to_owned();
            s.background.clone_from(&background);
            (
                src,
                title_changed.then(|| title.to_owned()),
                bg_changed.then_some(background),
            )
        };
        if src.is_none() && title_arg.is_none() && bg_arg.is_none() {
            return;
        }
        let js = |v: Option<String>| v.map_or_else(|| "null".to_owned(), |s| serde_json::Value::String(s).to_string());
        let _ = self.webview.evaluate_script(&format!(
            "window.__nodusFrame&&window.__nodusFrame({},{},{})",
            js(src),
            js(title_arg),
            js(bg_arg.clone())
        ));
        if let Some(bg) = bg_arg
            && let Some(c) = parse_hex(&bg)
        {
            let _ = self.webview.set_background_color(c);
        }
    }

    /// The page's `prefers-color-scheme` follows the app's theme (VIEW-026):
    /// on macOS through the app's appearance, which the webview inherits;
    /// WebView2 is told.
    pub fn set_dark(&self, dark: bool) {
        #[cfg(windows)]
        {
            use wry::WebViewExtWindows as _;
            let _ = self
                .webview
                .set_theme(if dark { wry::Theme::Dark } else { wry::Theme::Light });
        }
        #[cfg(not(windows))]
        let _ = dark;
    }

    /// Takes a picture of the page; `done` gets it on the main thread.
    pub fn snapshot(&self, done: impl FnOnce(Option<Arc<RenderImage>>) + 'static) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            super::mac::snapshot(&self.webview.webview(), move |pixels| {
                done(pixels.and_then(|(w, h, bgra)| {
                    if let Some(dir) = std::env::var_os("NODUS_VIEWS_SNAPSHOT_DIR") {
                        dump(&dir, w, h, &bgra);
                    }
                    let buf = image::RgbaImage::from_raw(w, h, bgra)?;
                    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
                }));
            });
        }
        #[cfg(not(target_os = "macos"))]
        done(None);
    }

    /// Whether the page has the window's keyboard (the user last clicked or
    /// typed in it, not elsewhere on the board).
    pub fn has_keyboard(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            super::mac::has_keyboard(&self.webview.webview())
        }
        // WebView2: the shell's own check (its frame has the focus) stands.
        #[cfg(not(target_os = "macos"))]
        true
    }

    /// Gives the keyboard back to the board's window.
    pub fn focus_parent(&self) {
        let _ = self.webview.focus_parent();
    }
}

/// Debugging: writes a snapshot as a PNG into `dir`.
#[cfg(target_os = "macos")]
fn dump(dir: &std::ffi::OsStr, w: u32, h: u32, bgra: &[u8]) {
    let rgba: Vec<u8> = bgra.chunks(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
    if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let _ = img.save(std::path::Path::new(dir).join(format!("snap-{n}-{w}x{h}.png")));
    }
}

fn rgba8(c: Rgba) -> (u8, u8, u8, u8) {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (ch(c.r), ch(c.g), ch(c.b), 255)
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.strip_prefix('#')?;
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

/// The views' scheme: this frame's shell, else the page as the core serves
/// it, status and headers as they are (VIEW-019, VIEW-020).
fn serve(nodus: &Nodus, shell_state: &Mutex<ShellState>, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let url = req.uri().to_string();
    let html = shell_state.lock().ok().and_then(|s| {
        shell::is_shell(&url, &s.view_id, s.frame)
            .then(|| shell::shell_html(&s.view_id, s.version, &s.title, &s.background))
    });
    if let Some(html) = html {
        return Response::builder()
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(html.into_bytes()))
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])));
    }
    let page = nodus.views().serve_page(&url);
    let mut builder = Response::builder().status(page.status);
    for (name, value) in &page.headers {
        builder = builder.header(*name, value.as_str());
    }
    builder
        .body(Cow::Owned(page.body.into_bytes()))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_colours_as_css_does() {
        assert_eq!(css_color(crate::theme::Tokens::LIGHT.bg_elevated), "#ffffff");
        assert_eq!(css_color(crate::theme::Tokens::DARK.bg_elevated), "#2a2a2d");
        assert_eq!(parse_hex("#2a2a2d"), Some((0x2a, 0x2a, 0x2d, 255)));
    }
}
