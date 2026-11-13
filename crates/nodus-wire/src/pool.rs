//! The pool of Claude accounts a server's sessions run on (SRV-100…105):
//! what members see of each account. Never its token: that goes in with
//! `accounts:add` and never comes back out (SRV-100).
//!
//! ```text
//! accounts:list   {}                → ClaudeAccount[]
//! accounts:add    { label, token }  → ClaudeAccount
//! accounts:rename { id, label }     → ClaudeAccount
//! accounts:remove { id }            → null
//! server → client  { "ev": "accounts-changed", "p": {} }
//! ```

use serde::{Deserialize, Serialize};

use crate::AuditMember;

/// Where an account stands (SRV-103).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountState {
    /// Sessions may start on it.
    #[default]
    Ok,
    /// At its usage limit until `resetsAt`, when it's `ok` again by itself.
    Limited,
    /// Claude turned it down (`authentication_failed`, `account_on_hold`,
    /// `billing_error`): it stays out until someone adds it again.
    Failed,
}

/// One of an account's usage windows, as Claude Code's status line last
/// reported it (`rate_limits.five_hour`, `rate_limits.seven_day`, SRV-102).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// 0 to 100.
    pub used_percentage: f64,
    /// When the window starts over, in milliseconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<f64>,
}

/// A Claude account in a server's pool, as members see it (SRV-100,
/// SRV-105): everything but its token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAccount {
    pub id: String,
    pub label: String,
    /// The member who added it; none for the development token.
    pub added_by: Option<AuditMember>,
    /// Milliseconds since the epoch.
    pub added_at: f64,
    pub state: AccountState,
    /// While `limited`: when it's `ok` again, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageWindow>,
    /// When a session or a background `claude -p` last took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<f64>,
    /// While `failed`: what Claude Code said (`authentication_failed`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// How many live sessions run on it now.
    #[serde(default)]
    pub sessions: u32,
}

/// A notice's kind: an account stopped working, `<label>'s Claude account
/// stopped working.` (SRV-103).
pub const ACCOUNT_FAILED: &str = "account-failed";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_everything_but_a_token() {
        let account = ClaudeAccount {
            id: "a1".into(),
            label: "Saleem's Max".into(),
            added_by: Some(AuditMember {
                github_id: 1,
                login: "octo".into(),
            }),
            added_at: 1.0,
            state: AccountState::Limited,
            resets_at: Some(2.0),
            five_hour: Some(UsageWindow {
                used_percentage: 100.0,
                resets_at: Some(2.0),
            }),
            seven_day: None,
            last_used_at: None,
            failure: None,
            sessions: 1,
        };
        assert_eq!(
            nodus_domain::json::to_string(&account).unwrap(),
            r#"{"id":"a1","label":"Saleem's Max","addedBy":{"githubId":1,"login":"octo"},"addedAt":1,"state":"limited","resetsAt":2,"fiveHour":{"usedPercentage":100,"resetsAt":2},"sessions":1}"#
        );
    }
}
