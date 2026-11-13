//! The Nodus servers the app connects to (SRV-160…164): how each one's
//! connection is doing, and what the app shows of it.

use serde::{Deserialize, Serialize};

string_enum! {
    /// How the app's connection to a server is doing (SRV-027, SRV-028).
    pub enum ServerState {
        /// The first attempt since the app started or the server was added.
        Connecting = "connecting",
        Connected = "connected",
        /// It can't be reached, or turned the token down; tried again after
        /// 1, 2, 4… seconds, at most every 30. Its boards show read-only.
        Offline = "offline",
        /// It speaks another protocol, so the two don't talk: `This server
        /// runs Nodus <version>. Update it in Settings → Servers.`
        ProtocolMismatch = "protocol-mismatch",
    }
}

/// A server, as the app shows it: what's saved of it, without its token,
/// and how its connection is doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub id: String,
    /// `http://host:port`, as it was added.
    pub url: String,
    /// The server's own name, from its hello.
    pub name: String,
    pub state: ServerState,
    /// The Nodus version it runs, once it has said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Why it's offline, when it is: the connection's error, or the token
    /// turned down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Who this device signed in as, by GitHub login, refreshed at each
    /// sign-in; none for a server added with a token (SRV-040).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// That member's GitHub id, which is what the server goes by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_id: Option<u64>,
}

/// How updating a server is going (SRV-200), as `ev:server-update` tells
/// every member's app, and as `server:update` answers when it's done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ServerUpdateStatus {
    /// Asking GitHub for the newest release.
    Checking,
    /// It runs the newest release already, `version`: nothing to do.
    Latest {
        version: String,
    },
    Downloading {
        version: String,
        percent: u8,
    },
    /// Installed: the server exits now, and systemd starts `version`.
    Restarting {
        version: String,
    },
    Error {
        message: String,
    },
}
