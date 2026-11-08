//! Voice prompts: speech to text through xAI.

use serde::{Deserialize, Serialize};

string_enum! {
    /// Where the xAI API key for voice prompts comes from. Nowhere yet is
    /// `None`, written as `null`.
    pub enum VoiceKeySource {
        /// Saved in Settings.
        Settings = "settings",
        /// The XAI_API_KEY in the user's shell.
        Env = "env",
    }
}

/// What a spoken prompt came back as. The two are told apart by their fields,
/// as the TypeScript's union is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum TranscribeResult {
    Text {
        text: String,
    },
    Failed {
        error: String,
        /// The API key is missing or was refused.
        #[serde(skip_serializing_if = "Option::is_none")]
        key_problem: Option<bool>,
    },
}
