//! The page zoom (spec 01 PLAT-057, PLAT-139): View › Actual Size, Zoom In
//! and Zoom Out set a zoom level, 0, +0.5 or −0.5 from the current one, and
//! the whole UI is drawn 1.2^level times larger. 0.9.4's Chromium kept the
//! level in `Preferences`; it's read from there once, the first time it's
//! asked for, and kept in state.json under `zoomLevel` from then on (docs/specs/README.md,
//! "What replaced Chromium"). 0.9.4 ignores the key.
//!
//! The zoom roles act on the page that has the keyboard. The capture panel's
//! page is the app's own with `#capture` after it, and Chromium keys a
//! `file://` page's zoom by its whole URL, so the panel has a level of its
//! own: zoomed while it has the keyboard, apart from the window's, brought
//! over and kept the same way, under `captureZoomLevel`.

use std::path::Path;

use serde_json::Value;

use crate::Nodus;

/// The top-level state.json key the level lives under.
pub const STATE_KEY: &str = "zoomLevel";
/// The key the capture panel's level lives under.
pub const CAPTURE_STATE_KEY: &str = "captureZoomLevel";

/// How the main window's page URL ends, as Chromium keyed its zoom.
const PAGE: &str = "/out/renderer/index.html";
/// How the capture panel's ends.
const CAPTURE_PAGE: &str = "/out/renderer/index.html#capture";

/// How far one Zoom In or Zoom Out moves the level (Electron's roles).
pub const STEP: f64 = 0.5;

/// Blink's page zoom limits: the level keeps counting past them, as
/// Electron's `zoomLevel += 0.5` does, but the page stops growing.
const MIN_FACTOR: f64 = 0.25;
const MAX_FACTOR: f64 = 5.0;

/// The UI's page zoom.
pub struct ZoomApi<'a> {
    pub(crate) nodus: &'a Nodus,
}

impl ZoomApi<'_> {
    /// The zoom level: 0 unless the user zoomed, here or in 0.9.4.
    pub fn level(&self) -> f64 {
        self.level_of(STATE_KEY, PAGE)
    }

    /// Sets the zoom level, saved a moment later. Returns it.
    pub fn set_level(&self, level: f64) -> f64 {
        self.set_level_of(STATE_KEY, PAGE, level)
    }

    /// The capture panel's zoom level: 0 unless the user zoomed the panel.
    pub fn capture_level(&self) -> f64 {
        self.level_of(CAPTURE_STATE_KEY, CAPTURE_PAGE)
    }

    /// Sets the capture panel's zoom level. Returns it.
    pub fn set_capture_level(&self, level: f64) -> f64 {
        self.set_level_of(CAPTURE_STATE_KEY, CAPTURE_PAGE, level)
    }

    fn level_of(&self, key: &str, page: &str) -> f64 {
        self.bring_over(key, page);
        self.nodus.inner.store.read(|state| {
            state
                .extra
                .get(key)
                .and_then(Value::as_f64)
                .filter(|l| l.is_finite())
                .unwrap_or(0.0)
        })
    }

    fn set_level_of(&self, key: &str, page: &str, level: f64) -> f64 {
        let level = if level.is_finite() { level } else { 0.0 };
        if self.level_of(key, page) != level {
            self.nodus.inner.store.update(|state| {
                state.extra.insert(key.to_owned(), Value::from(level));
            });
        }
        level
    }

    /// 0.9.4's level for the page whose URL ends with `page`, from
    /// Chromium's `Preferences`, while state.json has none of its own yet.
    /// A level of 0 is kept too, so it's read once.
    fn bring_over(&self, key: &str, page: &str) {
        let store = &self.nodus.inner.store;
        if store.read(|state| state.extra.contains_key(key)) {
            return;
        }
        let Some(user_data) = store.file().parent() else { return };
        let level = chromium_level_of(user_data, page).unwrap_or(0.0);
        store.update(|state| {
            state.extra.entry(key).or_insert_with(|| Value::from(level));
        });
    }
}

/// How much larger the UI is drawn at `level`: Chromium's `1.2^level`,
/// within Blink's limits.
pub fn factor(level: f64) -> f64 {
    1.2_f64.powf(level).clamp(MIN_FACTOR, MAX_FACTOR)
}

/// The level 0.9.4's page was zoomed to: `partition.per_host_zoom_levels`
/// in `<userData>/Preferences` holds, under the partition's hash, the
/// page's `file://…/out/renderer/index.html` and its level (a number, or an
/// object with `zoom_level` in newer Chromium). None when it never zoomed.
pub fn chromium_level(user_data: &Path) -> Option<f64> {
    chromium_level_of(user_data, PAGE)
}

/// The level Chromium kept for the app page whose `file://` URL ends with
/// `page` (a `file://` page's zoom is keyed by its whole URL, `#…` and all).
fn chromium_level_of(user_data: &Path, page: &str) -> Option<f64> {
    let text = std::fs::read(user_data.join("Preferences")).ok()?;
    let prefs: Value = serde_json::from_slice(&text).ok()?;
    let partitions = prefs.get("partition")?.get("per_host_zoom_levels")?.as_object()?;
    partitions
        .values()
        .filter_map(Value::as_object)
        .flat_map(|hosts| hosts.iter())
        .find(|(url, _)| url.starts_with("file://") && url.ends_with(page))
        .and_then(|(_, level)| match level {
            Value::Number(n) => n.as_f64(),
            Value::Object(o) => o.get("zoom_level").and_then(Value::as_f64),
            _ => None,
        })
        .filter(|l| l.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::nodus;

    fn preferences(dir: &Path, json: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("Preferences"), json).unwrap();
    }

    #[test]
    fn zooms_as_chromium_does() {
        assert_eq!(factor(0.0), 1.0);
        assert!((factor(-0.5) - 0.912_870_929).abs() < 1e-9);
        assert!((factor(0.5) - 1.095_445_115).abs() < 1e-9);
        // Blink stops at 25% and 500%.
        assert_eq!(factor(-20.0), 0.25);
        assert_eq!(factor(20.0), 5.0);
    }

    #[test]
    fn reads_the_level_chromium_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(chromium_level(dir.path()), None);
        preferences(
            dir.path(),
            r#"{"partition":{"per_host_zoom_levels":{"1234567890abcdef12":{"file:///Applications/Nodus.app/Contents/Resources/app.asar/out/renderer/index.html":-0.5}}}}"#,
        );
        assert_eq!(chromium_level(dir.path()), Some(-0.5));
        preferences(
            dir.path(),
            r#"{"partition":{"per_host_zoom_levels":{"x":{"file:///C:/Users/example/AppData/Local/Programs/nodus/resources/app.asar/out/renderer/index.html":{"last_modified":"1","zoom_level":1.0}}}}}"#,
        );
        assert_eq!(chromium_level(dir.path()), Some(1.0));
        preferences(
            dir.path(),
            r#"{"partition":{"per_host_zoom_levels":{"x":{"example.com":2.0}}}}"#,
        );
        assert_eq!(chromium_level(dir.path()), None);
    }

    #[tokio::test]
    async fn brings_the_level_over_once_and_keeps_its_own() {
        let (dir, nodus) = nodus();
        let user_data = dir.path().join("userData");
        preferences(
            &user_data,
            r#"{"partition":{"per_host_zoom_levels":{"h":{"file:///a/out/renderer/index.html":-0.5}}}}"#,
        );
        assert_eq!(nodus.zoom().level(), -0.5);
        assert_eq!(nodus.zoom().set_level(1.0), 1.0);
        assert_eq!(nodus.zoom().level(), 1.0);
        // Read once: Chromium's file no longer counts.
        preferences(
            &user_data,
            r#"{"partition":{"per_host_zoom_levels":{"h":{"file:///a/out/renderer/index.html":2.0}}}}"#,
        );
        assert_eq!(nodus.zoom().level(), 1.0);
        nodus.inner.store.flush();
        let saved: Value = serde_json::from_slice(&std::fs::read(user_data.join("state.json")).unwrap()).unwrap();
        assert_eq!(saved[STATE_KEY], 1.0);
    }

    #[tokio::test]
    async fn keeps_the_capture_panels_level_apart() {
        let (dir, nodus) = nodus();
        let user_data = dir.path().join("userData");
        // Chromium kept a file:// page's zoom under its whole URL: the panel's
        // `#capture` page has a key of its own beside the window's.
        preferences(
            &user_data,
            r#"{"partition":{"per_host_zoom_levels":{"h":{"file:///a/out/renderer/index.html":-0.5,"file:///a/out/renderer/index.html#capture":1.0}}}}"#,
        );
        assert_eq!(nodus.zoom().level(), -0.5);
        assert_eq!(nodus.zoom().capture_level(), 1.0);
        assert_eq!(nodus.zoom().set_capture_level(1.5), 1.5);
        assert_eq!(nodus.zoom().level(), -0.5);
        assert_eq!(nodus.zoom().set_level(0.0), 0.0);
        assert_eq!(nodus.zoom().capture_level(), 1.5);
        nodus.inner.store.flush();
        let saved: Value = serde_json::from_slice(&std::fs::read(user_data.join("state.json")).unwrap()).unwrap();
        assert_eq!(saved[CAPTURE_STATE_KEY], 1.5);
        assert_eq!(saved[STATE_KEY], 0.0);
    }

    #[tokio::test]
    async fn starts_at_actual_size_without_chromiums_file() {
        let (_dir, nodus) = nodus();
        assert_eq!(nodus.zoom().level(), 0.0);
    }
}
