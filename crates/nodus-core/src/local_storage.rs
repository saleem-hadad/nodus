//! The values 0.9.4's pages kept in Chromium's `localStorage` (the capture
//! draft, its attachments, sent-text history and the clipboard already
//! offered, CAP-157): per machine, across restarts. They live in state.json
//! under `localStorage`, each key holding the JSON text `localStorage.setItem`
//! held, so a page reads and writes them as 0.9.4's did (plan.md, "Decisions
//! for the user", the `localStorage` row). 0.9.4 ignores the key.

use serde_json::{Map, Value};

use crate::Nodus;

/// The top-level state.json key the values live under.
pub const STATE_KEY: &str = "localStorage";

/// A window's `localStorage`.
pub struct LocalStorageApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl LocalStorageApi<'_> {
    /// `localStorage.getItem(key)`: the text stored, or none.
    pub fn get(&self, key: &str) -> Option<String> {
        self.nodus.inner.store.read(|state| {
            state
                .extra
                .get(STATE_KEY)
                .and_then(|values| values.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    }

    /// `localStorage.setItem(key, text)`, or `removeItem(key)` with none.
    /// Saved a moment later, as every change to state.json is; unchanged
    /// values aren't written again.
    pub fn set(&self, key: &str, text: Option<&str>) {
        if self.get(key).as_deref() == text {
            return;
        }
        self.nodus.inner.store.update(|state| {
            let mut values = match state.extra.shift_remove(STATE_KEY) {
                Some(Value::Object(values)) => values,
                _ => Map::new(),
            };
            match text {
                Some(text) => {
                    values.insert(key.to_owned(), Value::String(text.to_owned()));
                }
                None => {
                    values.shift_remove(key);
                }
            }
            if !values.is_empty() {
                state.extra.insert(STATE_KEY.to_owned(), Value::Object(values));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::nodus;

    #[tokio::test]
    async fn keeps_what_a_page_stored_across_restarts() {
        let (dir, nodus) = nodus();
        let local = nodus.local_storage();
        assert_eq!(local.get("capture.draft"), None);
        local.set("capture.draft", Some("\"Fix the coupon field\""));
        local.set("capture.history", Some("[\"one\",\"two\"]"));
        assert_eq!(local.get("capture.draft").as_deref(), Some("\"Fix the coupon field\""));
        nodus.shutdown();
        drop(nodus);

        let text = std::fs::read_to_string(dir.path().join("userData").join("state.json")).unwrap();
        let state: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            state["localStorage"],
            serde_json::json!({ "capture.draft": "\"Fix the coupon field\"", "capture.history": "[\"one\",\"two\"]" })
        );

        let again = crate::Nodus::start(crate::NodusConfig {
            env: crate::EnvSource::Given(nodus_platform::Env::from_process()),
            home: dir.path().join("home"),
            ..crate::NodusConfig::new(dir.path().join("userData"), tokio::runtime::Handle::current())
        })
        .unwrap();
        assert_eq!(
            again.local_storage().get("capture.history").as_deref(),
            Some("[\"one\",\"two\"]")
        );
    }

    #[tokio::test]
    async fn removes_a_value_and_the_key_once_empty() {
        let (_dir, nodus) = nodus();
        let local = nodus.local_storage();
        local.set("capture.draft", Some("\"x\""));
        local.set("capture.draft", None);
        assert_eq!(local.get("capture.draft"), None);
        assert!(nodus.inner.store.read(|s| !s.extra.contains_key("localStorage")));
    }
}
