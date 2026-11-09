//! The shell page each frame's webview loads (spec 04, "wry: the frame"):
//! nothing but the view's page in `<iframe sandbox="allow-scripts">`, as
//! 0.9.4's board held it (VIEW-027), and a relay script. The page keeps its
//! opaque origin, `parent.postMessage` and `e.source === parent` work as they
//! did, and the shell tells the app what the page sent and whether the user
//! had just clicked or pressed something in it (VIEW-051).
//!
//! The shell is served on the views' own scheme at a path the page server
//! never answers (`/__nodus_shell`), once per frame, and only messages from
//! that exact document are taken (the page's own frame could reach the
//! webview's message handler directly; its messages are dropped).

/// The views' scheme (`VIEW_SCHEME`).
pub const SCHEME: &str = "nodus-view";
/// Where a frame's shell is served.
pub const SHELL_PATH: &str = "/__nodus_shell";

/// The page's origin for a view: `nodus-view://<id>` on macOS; WebView2
/// serves custom schemes as `https://nodus-view.<id>` (R11).
pub fn origin(view_id: &str) -> String {
    if cfg!(windows) {
        format!("https://{SCHEME}.{view_id}")
    } else {
        format!("{SCHEME}://{view_id}")
    }
}

/// The frame's `src` (VIEW-027): `nodus-view://<id>/?v=<version>`.
pub fn page_url(view_id: &str, version: u32) -> String {
    format!("{}/?v={version}", origin(view_id))
}

/// The shell for frame `frame` of a view.
pub fn shell_url(view_id: &str, frame: u64) -> String {
    format!("{}{SHELL_PATH}?f={frame}", origin(view_id))
}

/// The view a URL on the views' scheme is for, in either form.
pub fn view_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("nodus-view://")
        .or_else(|| url.strip_prefix("https://nodus-view."))
        .or_else(|| url.strip_prefix("http://nodus-view."))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    (!host.is_empty()).then_some(host)
}

/// Whether a URL is this frame's shell.
pub fn is_shell(url: &str, view_id: &str, frame: u64) -> bool {
    url == shell_url(view_id, frame)
}

/// VIEW-028: a frame only ever shows its own view's page: never the web,
/// never another view's, never a shell. `shell_pending` lets the webview's
/// own load of its shell through, once.
pub fn allow_navigation(url: &str, view_id: &str, frame: u64, shell_pending: &mut bool) -> bool {
    if is_shell(url, view_id, frame) {
        return std::mem::take(shell_pending);
    }
    if view_of(url) != Some(view_id) {
        return false;
    }
    let path = url
        .split_once("://")
        .map(|(_, rest)| rest.split_once('/').map_or("", |(_, p)| p))
        .unwrap_or_default();
    !path.starts_with(&SHELL_PATH[1..])
}

/// Escapes text for an HTML attribute value.
fn attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// The shell document: the frame filling it, on `background` (the card's
/// `--bg-elevated`, so a reload never flashes white).
pub fn shell_html(view_id: &str, version: u32, title: &str, background: &str) -> String {
    let frame_src = if cfg!(windows) {
        origin(view_id)
    } else {
        format!("{SCHEME}:")
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src {frame_src}"><style>html,body{{margin:0;width:100%;height:100%;overflow:hidden;background:{background}}}iframe{{position:absolute;top:0;left:0;width:100%;height:100%;border:0;display:block;background:{background}}}</style></head><body><iframe id="f" sandbox="allow-scripts" title="{title}" src="{src}"></iframe><script>{SCRIPT}</script></body></html>"#,
        title = attr(title),
        src = attr(&page_url(view_id, version)),
    )
}

/// The relay: messages from the frame go to the app with whether the user
/// had just clicked or pressed something in it; the app's go to the frame.
const SCRIPT: &str = r#"(function(){
var f=document.getElementById('f');
addEventListener('message',function(e){
if(e.source!==f.contentWindow)return;
var d=e.data;
if(!d||typeof d!=='object'||d.nodus!==1)return;
var ua=navigator.userActivation;
var activated=document.hasFocus()&&document.activeElement===f&&!!(ua&&ua.isActive);
var text;
try{text=JSON.stringify({m:d,activated:activated});}catch(err){return;}
window.ipc.postMessage(text);
});
f.addEventListener('mouseenter',function(){window.ipc.postMessage('{"h":true}');});
f.addEventListener('mouseleave',function(){window.ipc.postMessage('{"h":false}');});
window.__nodusDeliver=function(m){if(f.contentWindow)f.contentWindow.postMessage(m,'*');};
window.__nodusFrame=function(src,title,bg){
if(title!==null)f.title=title;
if(bg!==null){document.documentElement.style.background=bg;document.body.style.background=bg;f.style.background=bg;}
if(src!==null&&f.getAttribute('src')!==src)f.setAttribute('src',src);
};
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0f8e6b8a-4f2c-4b1e-9b3a-1c2d3e4f5a6b";

    #[test]
    fn builds_urls_on_the_views_scheme() {
        if cfg!(windows) {
            assert_eq!(page_url(ID, 3), format!("https://nodus-view.{ID}/?v=3"));
        } else {
            assert_eq!(page_url(ID, 3), format!("nodus-view://{ID}/?v=3"));
            assert_eq!(shell_url(ID, 7), format!("nodus-view://{ID}/__nodus_shell?f=7"));
        }
        assert_eq!(view_of(&page_url(ID, 1)), Some(ID));
        assert_eq!(view_of(&format!("https://nodus-view.{ID}/?v=2")), Some(ID));
        assert_eq!(view_of("https://example.com/"), None);
        assert_eq!(view_of("nodus-view:///"), None);
    }

    #[test]
    fn keeps_a_frame_on_its_own_page() {
        let mut pending = true;
        // The webview's own shell, once.
        assert!(allow_navigation(&shell_url(ID, 1), ID, 1, &mut pending));
        assert!(!allow_navigation(&shell_url(ID, 1), ID, 1, &mut pending));
        // The page, at any version; the same view's other paths 404 but may load.
        assert!(allow_navigation(&page_url(ID, 4), ID, 1, &mut pending));
        assert!(allow_navigation(&format!("{}/x", origin(ID)), ID, 1, &mut pending));
        // Never the web, another view, or a shell the page asks for.
        assert!(!allow_navigation("https://example.com/", ID, 1, &mut pending));
        assert!(!allow_navigation(
            &page_url("aaaaaaaa-4f2c-4b1e-9b3a-1c2d3e4f5a6b", 1),
            ID,
            1,
            &mut pending
        ));
        assert!(!allow_navigation(&shell_url(ID, 2), ID, 1, &mut pending));
        assert!(!allow_navigation("about:blank", ID, 1, &mut pending));
    }

    #[test]
    fn writes_the_frame_as_0_9_4_did() {
        let html = shell_html(ID, 3, "Docs \"build\" <status>", "#ffffff");
        assert!(
            html.contains(r#"<iframe id="f" sandbox="allow-scripts" title="Docs &quot;build&quot; &lt;status&gt;""#)
        );
        assert!(html.contains(&format!(r#"src="{}""#, page_url(ID, 3))));
        assert!(html.contains("background:#ffffff"));
        assert!(html.contains("default-src 'none'"));
    }
}
