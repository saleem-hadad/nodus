//! The values 0.9.4's pages kept in Chromium's `localStorage` (the capture
//! draft, its attachments, sent-text history and the clipboard already
//! offered, CAP-157): per machine, across restarts. They live in state.json
//! under `localStorage`, each key holding the JSON text `localStorage.setItem`
//! held, so a page reads and writes them as 0.9.4's did (docs/specs/README.md,
//! "What replaced Chromium"). 0.9.4 ignores the key.
//!
//! The first time a page reads one, what 0.9.4 left in Chromium's
//! `localStorage` for its pages (`file://`) is brought over ([`leveldb`]).
//! Once the key is in state.json it stays, empty or not, so that's once.

mod leveldb;

use serde_json::{Map, Value};

use crate::Nodus;

/// The top-level state.json key the values live under.
pub const STATE_KEY: &str = "localStorage";

/// The origin 0.9.4's packaged pages load from (`loadFile`).
const ORIGIN: &str = "file://";

/// A window's `localStorage`.
pub struct LocalStorageApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl LocalStorageApi<'_> {
    /// `localStorage.getItem(key)`: the text stored, or none.
    pub fn get(&self, key: &str) -> Option<String> {
        self.bring_over();
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
            let values = state
                .extra
                .entry(STATE_KEY)
                .or_insert_with(|| Value::Object(Map::new()));
            if !values.is_object() {
                *values = Value::Object(Map::new());
            }
            let Value::Object(values) = values else { return };
            match text {
                Some(text) => {
                    values.insert(key.to_owned(), Value::String(text.to_owned()));
                }
                None => {
                    values.shift_remove(key);
                }
            }
        });
    }

    /// 0.9.4's values, from Chromium's LevelDB, while state.json has none
    /// of its own yet. Nothing is written when there was nothing to bring.
    fn bring_over(&self) {
        let store = &self.nodus.inner.store;
        if store.read(|state| state.extra.contains_key(STATE_KEY)) {
            return;
        }
        let Some(user_data) = store.file().parent() else { return };
        let found = leveldb::read(&user_data.join("Local Storage").join("leveldb"), ORIGIN);
        if found.is_empty() {
            return;
        }
        store.update(|state| {
            if state.extra.contains_key(STATE_KEY) {
                return;
            }
            let values: Map<String, Value> = found.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
            state.extra.insert(STATE_KEY.to_owned(), Value::Object(values));
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
    async fn removes_a_value_and_keeps_the_key_once_empty() {
        let (_dir, nodus) = nodus();
        let local = nodus.local_storage();
        local.set("capture.draft", Some("\"x\""));
        local.set("capture.draft", None);
        assert_eq!(local.get("capture.draft"), None);
        assert_eq!(
            nodus.inner.store.read(|s| s.extra.get("localStorage").cloned()),
            Some(serde_json::json!({}))
        );
    }

    /// A LevelDB log holding one write batch that puts `values` under `origin`.
    fn chromium_log(origin: &str, values: &[(&str, &str)]) -> Vec<u8> {
        let mut batch = 1u64.to_le_bytes().to_vec();
        batch.extend_from_slice(&(values.len() as u32).to_le_bytes());
        for (key, value) in values {
            let mut k = vec![b'_'];
            k.extend_from_slice(origin.as_bytes());
            k.push(0);
            k.push(1);
            k.extend_from_slice(key.as_bytes());
            let mut v = vec![1];
            v.extend_from_slice(value.as_bytes());
            batch.push(1);
            batch.push(k.len() as u8);
            batch.extend_from_slice(&k);
            batch.push(v.len() as u8);
            batch.extend_from_slice(&v);
        }
        let mut log = vec![0, 0, 0, 0];
        log.extend_from_slice(&(batch.len() as u16).to_le_bytes());
        log.push(1);
        log.extend_from_slice(&batch);
        log
    }

    #[tokio::test]
    async fn brings_over_what_0_9_4_left_once() {
        let (dir, nodus) = nodus();
        let leveldb = dir.path().join("userData").join("Local Storage").join("leveldb");
        std::fs::create_dir_all(&leveldb).unwrap();
        std::fs::write(
            leveldb.join("000003.log"),
            chromium_log(
                "file://",
                &[
                    ("capture.draft", "\"Fix the coupon\""),
                    ("capture.history", "[\"one\"]"),
                ],
            ),
        )
        .unwrap();
        let local = nodus.local_storage();
        assert_eq!(local.get("capture.draft").as_deref(), Some("\"Fix the coupon\""));
        assert_eq!(local.get("capture.history").as_deref(), Some("[\"one\"]"));
        // Sent: the draft goes, and isn't brought over again.
        local.set("capture.draft", None);
        assert_eq!(local.get("capture.draft"), None);
    }

    #[tokio::test]
    async fn writes_nothing_when_there_is_nothing_to_bring() {
        let (_dir, nodus) = nodus();
        assert_eq!(nodus.local_storage().get("capture.draft"), None);
        assert!(nodus.inner.store.read(|s| !s.extra.contains_key("localStorage")));
    }
}
