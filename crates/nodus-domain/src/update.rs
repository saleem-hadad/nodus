//! How updating the app is going.

use serde::{Deserialize, Serialize};

/// How updating the app is going. `version` is the newer release, once one is
/// found. `Off` when this copy can't update itself: a development build, or
/// one in a folder it can't write to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum UpdateState {
    Off { reason: String },
    Idle,
    Checking,
    Latest,
    Downloading { version: String, percent: u8 },
    Ready { version: String },
    Error { message: String },
}

/// The update state, with the version running now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    #[serde(flatten)]
    pub state: UpdateState,
    pub current: String,
}
