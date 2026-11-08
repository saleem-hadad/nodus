//! A wry (WKWebView / WebView2) child of the GPUI window, following a board card.
//!
//! - built with `build_as_child(window)` (GPUI's `Window: HasWindowHandle` hands out its NSView);
//! - pages come from a custom protocol `nodus-view://<id>/` (Electron: protocol.handle);
//! - JS -> Rust over `window.ipc.postMessage`, Rust -> JS over `evaluate_script`;
//! - macOS: the WKWebView is re-parented into a layer-masked container NSView so it can be clipped
//!   to the board viewport without changing its document frame (no reflow while panning);
//! - board zoom maps to WKWebView pageZoom (`WebView::zoom`), frame = card body * zoom, so the CSS
//!   viewport stays the card's unscaled size, like the Electron iframe under a CSS transform.
use std::borrow::Cow;
use std::cell::Cell;

use gpui::*;
use wry::http::{Request, Response};
use wry::{WebViewBuilder, WebViewExtMacOS as _};

#[cfg(target_os = "macos")]
use crate::mac::ClipContainer;

/// The shell page the webview loads (custom scheme, main frame). It holds the view's page in a
/// `sandbox="allow-scripts"` iframe — same isolation as the 0.9.4 iframe — and relays messages
/// between the page (postMessage) and Rust (window.ipc / evaluate_script).
const SHELL: &str = r#"<!doctype html>
<html><head><meta charset="utf-8">
<style>html,body{margin:0;height:100%;overflow:hidden;background:transparent}iframe{border:0;width:100%;height:100%;display:block}</style>
</head><body>
<iframe id="f" sandbox="allow-scripts" src="nodus-view://demo/page"></iframe>
<script>
  const f = document.getElementById('f');
  const ua = () => navigator.userActivation
    ? { isActive: navigator.userActivation.isActive, hasBeenActive: navigator.userActivation.hasBeenActive }
    : 'unsupported';
  addEventListener('message', (e) => {
    if (e.source !== f.contentWindow) return;          // only our sandboxed child
    const msg = Object.assign({}, e.data, { origin: e.origin, shellActivation: ua() });
    window.ipc.postMessage(JSON.stringify(msg));
  });
  window.__nodusReply = (msg) => f.contentWindow.postMessage(msg, '*');
</script></body></html>"#;

const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8">
<meta name="color-scheme" content="light dark">
<style>
  :root { color-scheme: light dark; font: 13px -apple-system, system-ui, sans-serif; }
  body { margin: 0; padding: 44px 12px 12px; background: #ffffff; color: #1d1d1f; }
  @media (prefers-color-scheme: dark) { body { background: #2a2a2d; color: #f2f2f4; } }
  h1 { font-size: 15px; margin: 0 0 8px; }
  .bar { height: 10px; border-radius: 5px; background: linear-gradient(90deg,#007aff,#af52de); margin: 6px 0; }
  code { font: 12px ui-monospace, Menlo, monospace; }
  #log { white-space: pre-wrap; font: 11px ui-monospace, Menlo, monospace; opacity: .8 }
  #act { position: fixed; left: 8px; top: 8px; width: 130px; height: 28px; font: inherit; }
</style></head>
<body>
  <button id="act">start session</button>
  <h1>nodus-view://demo/page</h1>
  <div class="bar"></div>
  <div>viewport <code id="vp"></code> · dpr <code id="dpr"></code> · <code id="sch"></code> · origin <code id="org"></code></div>
  <div>IPC: <b id="ipc">waiting…</b></div>
  <div id="log"></div>
<script>
  const $ = (id) => document.getElementById(id);
  const ua = () => navigator.userActivation
    ? { isActive: navigator.userActivation.isActive, hasBeenActive: navigator.userActivation.hasBeenActive }
    : 'unsupported';
  const upd = () => {
    $('vp').textContent = innerWidth + '×' + innerHeight;
    $('dpr').textContent = devicePixelRatio.toFixed(2);
    $('sch').textContent = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    $('org').textContent = String(self.origin);
  };
  addEventListener('resize', upd); upd();
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', upd);
  const send = (m) => parent.postMessage(m, '*');     // the shell relays to Rust
  let n = 0;
  const ping = () => send({ type: 'ping', n: ++n, t: performance.now() });
  addEventListener('message', (e) => {
    const msg = e.data;
    $('ipc').textContent = 'ok · ' + JSON.stringify(msg);
    $('log').textContent += 'reply ' + msg.n + ' rtt ' + (performance.now() - msg.t).toFixed(1) + 'ms\n';
    if (msg.n < 3) setTimeout(ping, 50); else send({ type: 'done', n });
  });
  // "startSession only right after a click": report transient activation on click vs on a timer.
  $('act').addEventListener('click', () => send({ type: 'click', ua: ua() }));
  setTimeout(() => send({ type: 'timer', ua: ua() }), 600);
  setTimeout(ping, 100);
</script></body></html>"#;

fn serve(_id: wry::WebViewId, req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    log::info!("custom protocol: {}", req.uri());
    let (body, csp) = if req.uri().path().starts_with("/page") {
        // Electron view page CSP spirit: nothing but inline scripts/styles.
        (PAGE, "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'")
    } else {
        (SHELL, "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src nodus-view:")
    };
    Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Security-Policy", csp)
        .body(Cow::Borrowed(body.as_bytes()))
        .unwrap()
}

pub struct WebCard {
    pub webview: wry::WebView,
    #[cfg(target_os = "macos")]
    container: ClipContainer,
    zoom: Cell<f64>,
    pub hidden: Cell<bool>,
    pub last_doc: Cell<Bounds<Pixels>>,
    /// Last WKWebView snapshot, drawn by GPUI in place of the live view while it is obstructed.
    pub snapshot: std::cell::RefCell<Option<std::sync::Arc<RenderImage>>>,
    pub snapshot_pending: Cell<bool>,
    /// true while the live view is replaced by `snapshot`.
    pub showing_snapshot: Cell<bool>,
}

impl WebCard {
    pub fn new(window: &mut Window) -> anyhow::Result<(Self, smol::channel::Receiver<String>)> {
        let (tx, rx) = smol::channel::unbounded::<String>();
        let webview = WebViewBuilder::new()
            .with_custom_protocol("nodus-view".into(), serve)
            .with_url("nodus-view://demo/")
            .with_ipc_handler(move |req: Request<String>| {
                let _ = tx.try_send(req.into_body());
            })
            .with_accept_first_mouse(true)
            .build_as_child(&*window)?;

        #[cfg(target_os = "macos")]
        let container = {
            let parent = crate::mac::ns_view(window);
            let container = ClipContainer::new(&parent);
            let wk = webview.webview();
            wk.removeFromSuperview();
            container.view.addSubview(&wk);
            container
        };
        Ok((
            Self {
                webview,
                #[cfg(target_os = "macos")]
                container,
                zoom: Cell::new(1.0),
                hidden: Cell::new(false),
                last_doc: Cell::new(Bounds::default()),
                snapshot: Default::default(),
                snapshot_pending: Cell::new(false),
                showing_snapshot: Cell::new(false),
            },
            rx,
        ))
    }

    /// `document`: the card body in window coords; `clip`: the board viewport (content mask).
    pub fn place(&self, document: Bounds<Pixels>, clip: Bounds<Pixels>, zoom: f32, radius: f32) {
        self.last_doc.set(document);
        let visible = document.intersect(&clip);
        let hide = visible.size.width <= px(1.) || visible.size.height <= px(1.) || self.showing_snapshot.get();
        #[cfg(target_os = "macos")]
        {
            if hide != self.hidden.get() {
                self.container.set_hidden(hide);
                self.hidden.set(hide);
            }
            if !hide {
                let wk = self.webview.webview();
                self.container.set_geometry(&wk, visible, document, radius);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = self.webview.set_visible(!hide);
            let _ = self.webview.set_bounds(wry::Rect {
                position: wry::dpi::LogicalPosition::new(f64::from(document.origin.x), f64::from(document.origin.y)).into(),
                size: wry::dpi::LogicalSize::new(f64::from(document.size.width), f64::from(document.size.height)).into(),
            });
        }
        let z = zoom as f64;
        if (z - self.zoom.get()).abs() > 0.001 {
            self.zoom.set(z);
            let _ = self.webview.zoom(z);
        }
    }

    pub fn set_passthrough(&self, on: bool) {
        #[cfg(target_os = "macos")]
        self.container.set_passthrough(on);
    }

    /// Capture the page (async). `done` runs on the main thread with the GPUI image.
    pub fn request_snapshot(&self, done: impl FnOnce(Option<std::sync::Arc<RenderImage>>) + 'static) {
        if self.snapshot_pending.replace(true) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let wk = self.webview.webview();
            crate::mac::snapshot_wkwebview(&wk, move |px| {
                if let (Some((w, h, bgra)), Ok(path)) = (&px, std::env::var("S1_SNAPSHOT_PNG")) {
                    let rgba: Vec<u8> = bgra.chunks(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
                    if let Some(img) = image::RgbaImage::from_raw(*w, *h, rgba) {
                        let _ = img.save_with_format(&path, image::ImageFormat::Png);
                    }
                }
                let img = px.and_then(|(w, h, bgra)| {
                    let buf = image::RgbaImage::from_raw(w, h, bgra)?;
                    Some(std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
                });
                done(img);
            });
        }
    }
}
