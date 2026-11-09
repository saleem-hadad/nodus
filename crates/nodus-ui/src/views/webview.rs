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
    zoom: Cell<f64>,
    offscreen: Cell<bool>,
    live: Cell<bool>,
    /// WebKit scales the page by view scale (macOS); else by page zoom.
    #[cfg(target_os = "macos")]
    view_scale: bool,
    /// The page's size on screen, for its snapshot.
    #[cfg(target_os = "macos")]
    page: Cell<(Pixels, Pixels)>,
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
        let view_scale = {
            use wry::WebViewExtMacOS as _;
            super::mac::ignore_window_occlusion(&webview.webview());
            super::mac::set_view_scale(&webview.webview(), 1.0)
        };
        Ok(Self {
            frame,
            webview,
            #[cfg(target_os = "macos")]
            container,
            shell: shell_state,
            zoom: Cell::new(1.0),
            offscreen: Cell::new(true),
            live: Cell::new(true),
            #[cfg(target_os = "macos")]
            view_scale,
            #[cfg(target_os = "macos")]
            page: Cell::new((Pixels::ZERO, Pixels::ZERO)),
        })
    }

    /// Follows the card: `document` is the body on screen, `clip` what of it
    /// may show (the board's edges), `zoom` the board's zoom, which is the
    /// page's zoom too, so its CSS viewport stays the body's size in flow
    /// units. `radius` rounds the bottom corners.
    pub fn place(&self, document: Bounds<Pixels>, clip: Bounds<Pixels>, zoom: f32, radius: f32) {
        let visible = document.intersect(&clip);
        let offscreen = visible.size.width < gpui::px(1.) || visible.size.height < gpui::px(1.);
        self.offscreen.set(offscreen);
        self.set_zoom(f64::from(zoom));
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            self.container.set_hidden(offscreen);
            if !offscreen {
                let scale = if self.view_scale { f64::from(zoom) } else { 1.0 };
                self.page.set((document.size.width, document.size.height));
                self.container
                    .set_geometry(&self.webview.webview(), visible, document, radius, scale);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = radius;
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

    /// The page's scale: WKWebView's view scale (page zoom where WebKit has
    /// none), WebView2's `ZoomFactor`.
    pub fn set_zoom(&self, zoom: f64) {
        if (zoom - self.zoom.get()).abs() > 1e-4 {
            self.zoom.set(zoom);
            #[cfg(target_os = "macos")]
            if self.view_scale {
                use wry::WebViewExtMacOS as _;
                super::mac::set_view_scale(&self.webview.webview(), zoom);
                return;
            }
            let _ = self.webview.zoom(zoom);
        }
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

    /// Takes a picture of the page; `done` gets it on the main thread.
    pub fn snapshot(&self, done: impl FnOnce(Option<Arc<RenderImage>>) + 'static) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS as _;
            // With a view scale the page fills the webview's top left, at
            // the zoom's share of its size.
            let share = if self.view_scale { self.zoom.get().min(1.0) } else { 1.0 };
            super::mac::snapshot(&self.webview.webview(), None, move |pixels| {
                done(pixels.and_then(|(w, h, bgra)| {
                    let (cw, ch) = crop_size(w, h, share);
                    let bgra = crop_top_left(&bgra, w, cw, ch);
                    if let Some(dir) = std::env::var_os("NODUS_VIEWS_SNAPSHOT_DIR") {
                        dump(&dir, cw, ch, &bgra);
                    }
                    let buf = image::RgbaImage::from_raw(cw, ch, bgra)?;
                    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
                }));
            });
        }
        #[cfg(not(target_os = "macos"))]
        done(None);
    }

    /// Gives the keyboard back to the board's window.
    pub fn focus_parent(&self) {
        let _ = self.webview.focus_parent();
    }
}

/// The top-left `share` of a w×h picture, in whole pixels.
fn crop_size(w: u32, h: u32, share: f64) -> (u32, u32) {
    let cut = |v: u32| ((f64::from(v) * share).round() as u32).clamp(1, v.max(1));
    (cut(w), cut(h))
}

/// The top-left cw×ch of a BGRA picture `w` wide.
fn crop_top_left(bgra: &[u8], w: u32, cw: u32, ch: u32) -> Vec<u8> {
    if cw == w {
        return bgra[..(cw * ch * 4) as usize].to_vec();
    }
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for row in 0..ch as usize {
        let start = row * w as usize * 4;
        out.extend_from_slice(&bgra[start..start + cw as usize * 4]);
    }
    out
}

/// Debugging: writes a snapshot as a PNG into `dir`.
#[cfg(target_os = "macos")]
fn dump(dir: &std::ffi::OsStr, w: u32, h: u32, bgra: &[u8]) {
    let rgba: Vec<u8> = bgra.chunks(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
    if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let _ = img.save(std::path::Path::new(dir).join(format!("snap-{n}.png")));
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
