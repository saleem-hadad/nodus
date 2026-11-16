//! What a session card's drawer shows (spec 19): the pull requests its
//! session opened or works on, and the servers it started.
//!
//! A card keeps its pull requests with the fields this version doesn't know
//! (`prs` in `extra`), so they last until it's archived (DRAWER-013). Its
//! servers are only ever looked up, never kept (DRAWER-020).

use serde::{Deserialize, Serialize};

use crate::board::BoardNode;
use crate::hosted::{read, write};

string_enum! {
    /// Where a pull request stands.
    pub enum PrState {
        Open = "open",
        Draft = "draft",
        Merged = "merged",
        Closed = "closed",
    }
}

string_enum! {
    /// How the checks on a pull request's last commit went, all together.
    pub enum PrChecks {
        Pending = "pending",
        Passed = "passed",
        Failed = "failed",
    }
}

/// A pull request on a card (DRAWER-010…016).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardPr {
    /// Its page: `https://<host>/<owner>/<name>/pull/<number>`.
    pub url: String,
    pub number: u64,
    /// Empty until GitHub has been asked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// None until GitHub has been asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PrState>,
    /// None while it has no checks, or GitHub hasn't been asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<PrChecks>,
}

impl CardPr {
    /// One the session's transcript links to, before GitHub has been asked.
    pub fn found(url: &str) -> Option<Self> {
        let at = PrRef::parse(url)?;
        Some(Self {
            url: at.url(),
            number: at.number,
            title: String::new(),
            state: None,
            checks: None,
        })
    }
}

/// Something the session started that listens on a port (DRAWER-020…023).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardServer {
    pub port: u16,
    /// Its process, which Stop ends. None for a hosted card's preview port
    /// that nothing the session started holds (a container's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// What it runs, short: `vite`, `next dev`, `artisan serve`.
    pub label: String,
    /// When its process started, in ms since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
}

/// A pull request's place on GitHub, read from its link.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRef {
    /// `github.com`, or a GitHub Enterprise host.
    pub host: String,
    pub owner: String,
    pub name: String,
    pub number: u64,
}

impl PrRef {
    /// Reads `https://<host>/<owner>/<name>/pull/<number>`, with or without
    /// anything after the number (`/files`, `#issuecomment-…`).
    pub fn parse(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        let mut parts = rest.split('/');
        let host = parts.next().filter(|h| !h.is_empty())?;
        let owner = parts.next().filter(|s| is_name(s))?;
        let name = parts.next().filter(|s| is_name(s))?;
        if parts.next() != Some("pull") {
            return None;
        }
        let digits: String = parts.next()?.chars().take_while(char::is_ascii_digit).collect();
        let number = digits.parse().ok().filter(|&n| n > 0)?;
        Some(Self {
            host: host.to_ascii_lowercase(),
            owner: owner.to_owned(),
            name: name.to_owned(),
            number,
        })
    }

    /// Its link, without anything after the number.
    pub fn url(&self) -> String {
        format!(
            "https://{}/{}/{}/pull/{}",
            self.host, self.owner, self.name, self.number
        )
    }
}

/// An owner's or a repository's name as GitHub allows it.
fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl BoardNode {
    /// The pull requests the card has shown, as GitHub last described them
    /// (DRAWER-013). Kept until the card goes.
    pub fn prs(&self) -> Vec<CardPr> {
        read(&self.extra, "prs").unwrap_or_default()
    }

    pub fn set_prs(&mut self, prs: &[CardPr]) {
        write(&mut self.extra, "prs", (!prs.is_empty()).then_some(&prs));
    }
}

/// The card's pull requests: the ones it keeps, then the ones its transcript
/// links to that it doesn't keep yet, by link (DRAWER-011).
pub fn merge_prs(kept: Vec<CardPr>, found: &[String]) -> Vec<CardPr> {
    let mut prs = kept;
    for url in found {
        if let Some(pr) = CardPr::found(url)
            && !prs.iter().any(|p| p.url == pr.url)
        {
            prs.push(pr);
        }
    }
    prs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_pull_request_link() {
        let at = PrRef::parse("https://github.com/saleem-hadad/nodus/pull/53").unwrap();
        assert_eq!(
            (at.host.as_str(), at.owner.as_str(), at.name.as_str(), at.number),
            ("github.com", "saleem-hadad", "nodus", 53)
        );
        assert_eq!(PrRef::parse("https://github.com/a/b/pull/7/files").unwrap().number, 7);
        assert_eq!(
            PrRef::parse("https://github.com/a/b/pull/7#issuecomment-1")
                .unwrap()
                .url(),
            "https://github.com/a/b/pull/7"
        );
        assert_eq!(
            PrRef::parse("https://GHE.example.com/a/b.c/pull/12").unwrap().host,
            "ghe.example.com"
        );
        assert_eq!(PrRef::parse("https://github.com/a/b/issues/7"), None);
        assert_eq!(PrRef::parse("https://github.com/a/b/pull/"), None);
        assert_eq!(PrRef::parse("https://github.com/a/b/pull/0"), None);
        assert_eq!(PrRef::parse("github.com/a/b/pull/3"), None);
    }

    #[test]
    fn a_card_keeps_its_pull_requests_in_extra() {
        let mut node: BoardNode =
            serde_json::from_str(r#"{"sessionId":"s","x":0,"y":0,"archived":false,"createdAt":1}"#).unwrap();
        assert!(node.prs().is_empty());
        let pr = CardPr {
            url: "https://github.com/a/b/pull/3".into(),
            number: 3,
            title: "Fix it".into(),
            state: Some(PrState::Merged),
            checks: Some(PrChecks::Passed),
        };
        node.set_prs(std::slice::from_ref(&pr));
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""prs":[{"url":"https://github.com/a/b/pull/3","number":3,"title":"Fix it","state":"merged","checks":"passed"}]"#), "{json}");
        let back: BoardNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prs(), vec![pr]);
        node.set_prs(&[]);
        assert!(!node.extra.contains_key("prs"));
    }

    #[test]
    fn found_links_join_the_kept_ones_once() {
        let kept = vec![CardPr {
            title: "Kept".into(),
            ..CardPr::found("https://github.com/a/b/pull/1").unwrap()
        }];
        let found = [
            "https://github.com/a/b/pull/1".to_owned(),
            "https://github.com/a/b/pull/2/files".to_owned(),
            "https://github.com/a/b/pull/2".to_owned(),
            "not a link".to_owned(),
        ];
        let prs = merge_prs(kept, &found);
        assert_eq!(
            prs.iter().map(|p| (p.number, p.title.as_str())).collect::<Vec<_>>(),
            [(1, "Kept"), (2, "")]
        );
    }
}
