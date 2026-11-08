//! `window.api.settings`: the app-wide preferences.

use nodus_domain::{AppSettings, SettingsPatch};
use nodus_store::normalize_settings;
use serde_json::Value;

use crate::Nodus;

/// `window.api.settings`.
pub struct SettingsApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl SettingsApi<'_> {
    /// `settings:get` (PLAT-064).
    pub async fn get(&self) -> AppSettings {
        self.nodus.inner.store.read(|state| state.settings.clone())
    }

    /// `settings:update` (PLAT-065): the change over the current settings,
    /// normalized, applied (PLAT-066), saved, and given back whole.
    pub async fn update(&self, patch: SettingsPatch) -> AppSettings {
        let patch = serde_json::to_value(&patch).unwrap_or(Value::Null);
        self.update_json(patch).await
    }

    /// [`Self::update`] with the patch as the JSON the IPC carried: a value
    /// this version doesn't understand (`theme: 'blue'`, `autoUpdate: 0`) is
    /// read as normalizeSettings reads it, and keys it doesn't know are kept.
    pub async fn update_json(&self, patch: Value) -> AppSettings {
        let settings = self.nodus.inner.store.update(|state| {
            let mut merged = match serde_json::to_value(&state.settings) {
                Ok(Value::Object(current)) => current,
                _ => serde_json::Map::new(),
            };
            if let Value::Object(patch) = &patch {
                for (key, value) in patch {
                    merged.insert(key.clone(), value.clone());
                }
            }
            state.settings = normalize_settings(&Value::Object(merged));
            state.settings.clone()
        });
        self.nodus.apply_settings(&settings);
        // PLAT-065 step 4, UPD-013: turning automatic updates on checks now.
        self.nodus.settings_updated(&patch);
        settings
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::nodus;
    use nodus_domain::{SettingsPatch, ThemePreference};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn merges_normalizes_applies_and_saves() {
        let (_dir, nodus) = nodus();
        let applied = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&applied);
        nodus.on_settings_applied(Arc::new(move |s| seen.lock().unwrap().push(s.theme)));
        let settings = nodus
            .settings()
            .update(SettingsPatch {
                theme: Some(ThemePreference::Dark),
                focus_feature: Some(true),
                ..SettingsPatch::default()
            })
            .await;
        assert_eq!(settings.theme, ThemePreference::Dark);
        assert!(settings.focus_feature && settings.auto_update);
        assert_eq!(nodus.settings().get().await, settings);
        assert_eq!(
            *applied.lock().unwrap(),
            vec![ThemePreference::System, ThemePreference::Dark]
        );
    }

    #[tokio::test]
    async fn reads_a_bad_value_as_normalize_settings_does() {
        let (_dir, nodus) = nodus();
        nodus.settings().update_json(json!({ "theme": "dark" })).await;
        let settings = nodus
            .settings()
            .update_json(json!({ "theme": "blue", "autoUpdate": 0, "focusMode": 1 }))
            .await;
        assert_eq!(settings.theme, ThemePreference::System);
        assert!(settings.auto_update);
        assert!(!settings.focus_mode);
    }
}
