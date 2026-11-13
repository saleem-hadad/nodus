//! Making the GitHub App through GitHub's manifest flow (SRV-062): the page
//! that posts the manifest to GitHub, and the callback GitHub sends the
//! browser back to. Both are opened in the person's browser, so they answer
//! with pages, not JSON.

use std::collections::HashMap;

use nodus_wire::GITHUB_CALLBACK_PATH;
use serde_json::{Value, json};

use crate::auth::{Auth, is_login};

/// GitHub's longest App name.
const APP_NAME_CHARS: usize = 34;

/// `GET /github/setup/start?nonce=…&owner=<org or empty>`: a page that posts
/// the manifest to GitHub, with the nonce as `state`. `host` is the `Host`
/// the browser called this server by, already one of its own names
/// (SRV-021), so GitHub sends it back to an address it reaches.
pub(crate) fn start(auth: &Auth, query: &HashMap<String, String>, host: &str) -> (u16, String) {
    let nonce = query.get("nonce").map_or("", String::as_str);
    let Some(workflows) = auth.peek_nonce(nonce) else {
        return (400, refused_page());
    };
    let owner = query.get("owner").map_or("", |owner| owner.trim());
    if !owner.is_empty() && !is_login(owner) {
        return (
            400,
            page("Not an organization", "That isn't a GitHub organization's name."),
        );
    }
    let redirect = format!("http://{host}{GITHUB_CALLBACK_PATH}");
    let manifest = manifest(&auth.name(), &redirect, workflows);
    let target = match owner {
        "" => format!("{}/settings/apps/new", auth.github.web()),
        org => format!("{}/organizations/{org}/settings/apps/new", auth.github.web()),
    };
    let action = format!("{target}?state={nonce}");
    let body = format!(
        r#"<p>Taking you to GitHub to make this server's App…</p>
<form id="manifest" method="post" action="{action}">
<input type="hidden" name="manifest" value="{manifest}">
<noscript><button type="submit">Continue to GitHub</button></noscript>
</form>
<script>document.getElementById("manifest").submit();</script>"#,
        action = escape(&action),
        manifest = escape(&manifest.to_string()),
    );
    (200, wrap("Making the GitHub App", &body))
}

/// The manifest (SRV-062): a private App named `Nodus · <server name>`, with
/// contents, pull requests and issues to write, checks, actions and
/// metadata to read, workflows only when asked, and no webhook.
pub(crate) fn manifest(server_name: &str, redirect_url: &str, workflows: bool) -> Value {
    let name: String = format!("Nodus · {server_name}").chars().take(APP_NAME_CHARS).collect();
    let mut permissions = json!({
        "contents": "write",
        "pull_requests": "write",
        "issues": "write",
        "checks": "read",
        "actions": "read",
        "metadata": "read",
    });
    if workflows {
        permissions["workflows"] = json!("write");
    }
    // No `hook_attributes`: no webhook (SRV-062).
    json!({
        "name": name.trim_end(),
        "url": "https://github.com/saleem-hadad/nodus",
        "description": "Lets a Nodus server's sessions clone, push and open pull requests.",
        "public": false,
        "redirect_url": redirect_url,
        "default_permissions": permissions,
        "default_events": [],
        "request_oauth_on_install": false,
    })
}

/// `GET /github/setup/callback?code=…&state=…`: refused unless `state` is a
/// nonce this server gave and nobody has used (SRV-062); otherwise GitHub's
/// code is exchanged for the App, which is kept, and the page says what's
/// next. No secret is in the page.
pub(crate) async fn callback(auth: &Auth, query: &HashMap<String, String>) -> (u16, String) {
    let state = query.get("state").map_or("", String::as_str);
    // Used up before anything else, so the same state can't come twice.
    let Some((workflows, who)) = auth.take_nonce(state) else {
        return (400, refused_page());
    };
    let code = query.get("code").map_or("", String::as_str);
    if code.is_empty()
        || code.len() > 128
        || !code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return (
            400,
            page(
                "No App was made",
                "GitHub didn't send back a code. Start again from Nodus.",
            ),
        );
    }
    let made = match auth.github.convert_manifest(code).await {
        Ok(made) => made,
        Err(err) => return (502, page("No App was made", &format!("{err}. Start again from Nodus."))),
    };
    match auth.keep_app(&who, made, workflows) {
        Ok(app) => {
            let install = format!("{}/apps/{}/installations/new", auth.github.web(), app.slug);
            let body = format!(
                r#"<p><strong>{name}</strong> is made. Go back to Nodus to install it on repositories.</p>
<p>Install it only on repositories you'd trust every session with, never on ones that take outside contributions.</p>
<p><a href="{install}">Install on repositories</a></p>"#,
                name = escape(&app.name),
                install = escape(&install),
            );
            (200, wrap("GitHub App made", &body))
        }
        Err(err) => (500, page("The App can't be kept", &err)),
    }
}

/// What a missing, wrong, used or stale `state` or nonce gets.
fn refused_page() -> String {
    page(
        "This link doesn't work",
        "It has expired or was used already. Start again from Nodus.",
    )
}

fn page(title: &str, text: &str) -> String {
    wrap(title, &format!("<p>{}</p>", escape(text)))
}

fn wrap(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Nodus</title>
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:36rem;margin:4rem auto;padding:0 1rem;color:#222}}@media(prefers-color-scheme:dark){{body{{background:#1b1b1b;color:#ddd}}a{{color:#8ab4ff}}}}</style>
</head><body><h1>{title}</h1>
{body}
</body></html>
"#,
        title = escape(title),
    )
}

/// Text for HTML, in an element or a quoted attribute.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A query string's values, decoded; the last of a name wins.
pub(crate) fn query(uri: &hyper::Uri) -> HashMap<String, String> {
    url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_what_sessions_need_and_workflows_only_when_asked() {
        let plain = manifest("box", "http://box:7420/github/setup/callback", false);
        assert_eq!(plain["name"], "Nodus · box");
        assert_eq!(plain["public"], false);
        assert_eq!(plain["redirect_url"], "http://box:7420/github/setup/callback");
        assert_eq!(
            plain["default_permissions"],
            json!({
                "contents": "write",
                "pull_requests": "write",
                "issues": "write",
                "checks": "read",
                "actions": "read",
                "metadata": "read",
            })
        );
        assert!(plain.get("hook_attributes").is_none());
        let with = manifest("box", "http://box:7420/github/setup/callback", true);
        assert_eq!(with["default_permissions"]["workflows"], "write");
        // GitHub takes 34 characters at most.
        let long = manifest(&"x".repeat(60), "", false);
        assert_eq!(long["name"].as_str().unwrap().chars().count(), APP_NAME_CHARS);
    }

    #[test]
    fn escapes_what_goes_in_a_page() {
        assert_eq!(
            escape(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }
}
