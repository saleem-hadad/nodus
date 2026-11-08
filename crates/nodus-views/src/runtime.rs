//! A view's files in the data folder: `<userData>/views/<id>/index.html`, the
//! page as Claude sent it, and `runtime.json`, what it holds at run time
//! (VIEW-001 to VIEW-008).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nodus_domain::{CommandResult, json};
use serde_json::{Map, Value};

use crate::ViewError;
use crate::check::is_view_id;

/// A JavaScript error a view's page hit.
#[derive(Debug, Clone, PartialEq)]
pub struct PageError {
    /// At most 2000 UTF-16 units.
    pub message: String,
    pub at: f64,
}

/// What a view holds at run time, kept in `runtime.json`. None is
/// `undefined`, which isn't written; `Some(Null)` is `null`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Runtime {
    /// What Claude sent as the page's data (`nodus.data`).
    pub data: Option<Value>,
    /// What the page saved with `setState` (`nodus.state`).
    pub state: Option<Value>,
    /// Each command's last result, by its name.
    pub results: Map<String, Value>,
    /// The page's last 20 errors, oldest first.
    pub errors: Vec<PageError>,
    /// When its brain was last asked to refresh it.
    pub refreshed_at: Option<f64>,
}

impl Runtime {
    /// Reads what `runtime.json` held, taking what's usable: `results` only
    /// if it's an object, `errors` only if a list, `refreshedAt` only if a
    /// number (VIEW-005).
    pub fn from_json(raw: &Value) -> Self {
        let Value::Object(raw) = raw else {
            return Self::default();
        };
        let errors = match raw.get("errors") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|e| {
                    Some(PageError {
                        message: e.get("message")?.as_str()?.to_owned(),
                        at: e.get("at")?.as_f64()?,
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        Self {
            data: raw.get("data").cloned(),
            state: raw.get("state").cloned(),
            results: match raw.get("results") {
                Some(Value::Object(results)) => results.clone(),
                _ => Map::new(),
            },
            errors,
            refreshed_at: raw.get("refreshedAt").and_then(Value::as_f64),
        }
    }

    /// The runtime as `JSON.stringify` writes it.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(data) = &self.data {
            out.insert("data".into(), data.clone());
        }
        if let Some(state) = &self.state {
            out.insert("state".into(), state.clone());
        }
        out.insert("results".into(), Value::Object(self.results.clone()));
        let errors = self
            .errors
            .iter()
            .map(|e| serde_json::json!({ "message": e.message, "at": e.at }))
            .collect();
        out.insert("errors".into(), Value::Array(errors));
        if let Some(at) = self.refreshed_at {
            out.insert("refreshedAt".into(), Value::from(at));
        }
        Value::Object(out)
    }

    /// The last result of the command called `name`.
    pub fn result(&self, name: &str) -> Option<CommandResult> {
        serde_json::from_value(self.results.get(name)?.clone()).ok()
    }
}

/// Where a view's files go: `<userData>/views/<id>`, for an id that's a view id (VIEW-001, VIEW-002).
pub fn view_dir(user_data: &Path, view_id: &str) -> Result<PathBuf, ViewError> {
    // The id ends up in a path.
    if !is_view_id(view_id) {
        return Err(ViewError::new("Not a view id"));
    }
    Ok(user_data.join("views").join(view_id))
}

/// The page, or `''` when there's none or it can't be read (VIEW-003).
pub fn read_html(user_data: &Path, view_id: &str) -> String {
    view_dir(user_data, view_id)
        .ok()
        .and_then(|dir| fs::read(dir.join("index.html")).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// Writes the page as it is, making the folder first (VIEW-004).
pub fn write_html(user_data: &Path, view_id: &str, html: &str) -> Result<(), ViewError> {
    let dir = view_dir(user_data, view_id)?;
    fs::create_dir_all(&dir).map_err(|err| ViewError::new(fs_message(&err, "mkdir", &dir)))?;
    let file = dir.join("index.html");
    fs::write(&file, html).map_err(|err| ViewError::new(fs_message(&err, "open", &file)))
}

/// Reads `runtime.json`; an unreadable or invalid one is an empty runtime (VIEW-005).
pub fn read_runtime(user_data: &Path, view_id: &str) -> Runtime {
    let read = || -> Option<Runtime> {
        let bytes = fs::read(view_dir(user_data, view_id).ok()?.join("runtime.json")).ok()?;
        let raw: Value = json::from_str(&String::from_utf8_lossy(&bytes)).ok()?;
        Some(Runtime::from_json(&raw))
    };
    read().unwrap_or_default()
}

/// Writes `runtime.json` as compact JSON through `runtime.json.<pid>.tmp`
/// and a rename, so it's never half written (VIEW-006).
pub fn write_runtime(user_data: &Path, view_id: &str, runtime: &Runtime) -> io::Result<()> {
    let dir = view_dir(user_data, view_id).map_err(io::Error::other)?;
    fs::create_dir_all(&dir)?;
    let file = dir.join("runtime.json");
    let tmp = dir.join(format!("runtime.json.{}.tmp", std::process::id()));
    fs::write(&tmp, json::to_string(&runtime.to_json()).map_err(io::Error::other)?)?;
    fs::rename(&tmp, &file)
}

/// Removes a view's folder and all in it; one that isn't there is fine (VIEW-008).
pub fn delete_dir(user_data: &Path, view_id: &str) -> io::Result<()> {
    let dir = view_dir(user_data, view_id).map_err(io::Error::other)?;
    match fs::remove_dir_all(&dir) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// `EACCES: permission denied, open '<path>'`, as Node words it.
pub(crate) fn fs_message(err: &io::Error, syscall: &str, path: &Path) -> String {
    let named = match err.kind() {
        io::ErrorKind::NotFound => Some(("ENOENT", "no such file or directory")),
        io::ErrorKind::PermissionDenied => Some(("EACCES", "permission denied")),
        io::ErrorKind::AlreadyExists => Some(("EEXIST", "file already exists")),
        io::ErrorKind::NotADirectory => Some(("ENOTDIR", "not a directory")),
        io::ErrorKind::IsADirectory => Some(("EISDIR", "illegal operation on a directory")),
        io::ErrorKind::ReadOnlyFilesystem => Some(("EROFS", "read-only file system")),
        io::ErrorKind::StorageFull => Some(("ENOSPC", "no space left on device")),
        _ => None,
    };
    match named {
        Some((code, description)) => format!("{code}: {description}, {syscall} '{}'", path.display()),
        None => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

    #[test]
    fn reads_what_it_can_use_and_writes_it_back() {
        let raw = json!({
            "data": null, "results": [], "errors": [{ "message": "x", "at": 5 }, "junk"], "refreshedAt": "soon"
        });
        let rt = Runtime::from_json(&raw);
        assert_eq!(rt.data, Some(Value::Null));
        assert_eq!(rt.state, None);
        assert!(rt.results.is_empty());
        assert_eq!(
            rt.errors,
            [PageError {
                message: "x".into(),
                at: 5.0
            }]
        );
        assert_eq!(rt.refreshed_at, None);
        assert_eq!(
            json::to_string(&rt.to_json()).unwrap(),
            r#"{"data":null,"results":{},"errors":[{"message":"x","at":5}]}"#
        );
        assert_eq!(Runtime::from_json(&json!("nope")), Runtime::default());
    }

    #[test]
    fn keeps_files_in_the_views_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_html(dir.path(), ID), "");
        assert_eq!(read_html(dir.path(), "../x"), "");
        assert_eq!(
            write_html(dir.path(), "../x", "x").unwrap_err().to_string(),
            "Not a view id"
        );
        write_html(dir.path(), ID, "<p>é</p>").unwrap();
        assert_eq!(read_html(dir.path(), ID), "<p>é</p>");
        let rt = Runtime {
            refreshed_at: Some(1_700_000_000_000.0),
            ..Runtime::default()
        };
        write_runtime(dir.path(), ID, &rt).unwrap();
        let text = fs::read_to_string(dir.path().join("views").join(ID).join("runtime.json")).unwrap();
        assert_eq!(text, r#"{"results":{},"errors":[],"refreshedAt":1700000000000}"#);
        assert_eq!(read_runtime(dir.path(), ID), rt);
        fs::write(dir.path().join("views").join(ID).join("runtime.json"), "{").unwrap();
        assert_eq!(read_runtime(dir.path(), ID), Runtime::default());
        delete_dir(dir.path(), ID).unwrap();
        assert!(!dir.path().join("views").join(ID).exists());
        delete_dir(dir.path(), ID).unwrap();
    }
}
