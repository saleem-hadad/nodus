//! Answering the frame's request for `nodus-view://<id>/`: what the webview's
//! custom-protocol handler serves (VIEW-018 to VIEW-021).

use crate::page::{VIEW_CSP, VIEW_SCHEME};

/// What a request on the views' scheme gets back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageResponse {
    pub status: u16,
    /// In order, exactly these.
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl PageResponse {
    /// `200` with the page and exactly the headers of VIEW-020.
    pub(crate) fn page(body: String) -> Self {
        Self {
            status: 200,
            headers: vec![
                ("Content-Type", "text/html; charset=utf-8".into()),
                ("Content-Security-Policy", VIEW_CSP.into()),
                ("Cache-Control", "no-store".into()),
                ("X-Content-Type-Options", "nosniff".into()),
            ],
            body,
        }
    }

    /// `404 Not found`, as `new Response('Not found', {status: 404})` is.
    pub(crate) fn not_found() -> Self {
        Self {
            status: 404,
            headers: vec![("Content-Type", "text/plain;charset=UTF-8".into())],
            body: "Not found".into(),
        }
    }

    /// The value of a header, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The view a request is for: the URL's hostname, when its path is exactly
/// `/` once `.` and `..` are taken out; the query doesn't matter (VIEW-019).
///
/// Takes `nodus-view://<id>/`, and the form WebView2 gives custom schemes on
/// Windows, `http(s)://nodus-view.<id>/`.
pub fn view_id_from_url(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    let special = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    if !special && !scheme.eq_ignore_ascii_case(VIEW_SCHEME) {
        return None;
    }
    // The authority runs to the path, the query or the fragment.
    let ends = |c: char| matches!(c, '/' | '?' | '#') || (special && c == '\\');
    let end = rest.find(ends).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _)| host);
    let id = if special {
        host.strip_prefix(&format!("{VIEW_SCHEME}."))?
    } else {
        host
    };
    let path = tail.split(['?', '#']).next().unwrap_or_default();
    let path = if special {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    (normalize_path(&path) == "/").then_some(id)
}

/// The path with `.` and `..` segments taken out, as a URL parser does
/// (`%2e` counts as a dot). No path is `/`.
fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".into();
    }
    let single = |s: &str| s == "." || s.eq_ignore_ascii_case("%2e");
    let double = |s: &str| matches!(s.to_ascii_lowercase().as_str(), ".." | ".%2e" | "%2e." | "%2e%2e");
    let segments: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        if double(segment) {
            out.pop();
            if last {
                out.push("");
            }
        } else if single(segment) {
            if last {
                out.push("");
            }
        } else {
            out.push(segment);
        }
    }
    format!("/{}", out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

    #[test]
    fn takes_the_view_id_from_the_host_and_only_the_root_path() {
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/")), Some(ID));
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/?v=3")), Some(ID));
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/#top")), Some(ID));
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/./")), Some(ID));
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/x/..")), Some(ID));
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/../state.json")), None);
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/%2e%2e/state.json")), None);
        assert_eq!(view_id_from_url(&format!("nodus-view://{ID}/index.html")), None);
        assert_eq!(view_id_from_url(&format!("http://nodus-view.{ID}/?v=1")), Some(ID));
        assert_eq!(view_id_from_url(&format!("https://nodus-view.{ID}/")), Some(ID));
        assert_eq!(view_id_from_url(&format!("https://{ID}/")), None);
        assert_eq!(view_id_from_url(&format!("file://{ID}/")), None);
        assert_eq!(view_id_from_url("nodus-view:nothing"), None);
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
        assert_eq!(normalize_path("/.."), "/");
        assert_eq!(normalize_path("/a/."), "/a/");
        assert_eq!(normalize_path("/a/%2E%2e"), "/");
    }
}
