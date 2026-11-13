//! What state.json holds: the projects, their boards and the app's settings.

use indexmap::IndexMap;
use nodus_domain::{
    AppSettings, BoardLink, BoardNode, BoardNote, BoardSection, BoardShape, BoardView, CaptureKind, InkStroke, Project,
    ProjectAction, ProjectGroup, Rect, ThemePreference, Viewport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::records::Records;

/// Six 288px card columns with 24px gaps and 40px side padding.
pub const DEFAULT_ARCHIVE_W: f64 = 2.0 * 40.0 + 6.0 * 288.0 + 5.0 * 24.0;

/// Everything the app keeps in state.json.
///
/// The TypeScript drops top-level fields it doesn't know when it loads the
/// file. This keeps them in `extra`, as the types inside keep theirs, so a
/// file written by a newer version is never cut down by an older one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub projects: Vec<Project>,
    pub groups: Vec<ProjectGroup>,
    pub boards: IndexMap<String, BoardState>,
    pub settings: AppSettings,
    /// Global actions, in every project's actions rail. A project's own are on the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ProjectAction>>,
    /// The xAI API key for voice prompts, encrypted with Electron's safeStorage (base64). Never shown in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_key: Option<String>,
    /// What floating capture made lately, newest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captures: Option<Vec<CaptureRecord>>,
    /// The Nodus servers this app connects to, in the order they were added
    /// (SRV-161). Added after 0.9.4; left out while there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<ServerRecord>,
    /// What's this person's own on the boards of server projects, by project
    /// id: kept here, never on the server (SRV-164).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub server_boards: IndexMap<String, ServerBoardState>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A server this app connects to (SRV-161).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRecord {
    pub id: String,
    /// `http://host:port`, as it was added.
    pub url: String,
    /// The server's name, from its hello when it was added.
    #[serde(default)]
    pub name: String,
    /// This device's token for it, sealed with safeStorage as the voice key
    /// is, in base64. Never shown.
    #[serde(default)]
    pub token: String,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A server project's board, as far as it's this person's own (SRV-164):
/// where they last looked, and which cards they have open in place. The rest
/// of the board is the server's, shared with everyone on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerBoardState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// The cards expanded here, by session id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<String>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A project's board as it's stored.
///
/// The TypeScript adds `hidden`, `sections`, `notes` and `views` to a board
/// the first time it reads it; here they're there from the start, in the
/// order it puts them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardState {
    pub nodes: IndexMap<String, BoardNode>,
    pub archive: Rect,
    /// Sessions removed from the board; never re-imported.
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub sections: IndexMap<String, BoardSection>,
    #[serde(default)]
    pub notes: IndexMap<String, BoardNote>,
    #[serde(default)]
    pub views: IndexMap<String, BoardView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<Viewport>,
    /// Ink drawn on the board, oldest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ink: Option<Vec<InkStroke>>,
    /// Shapes on the board, oldest first. Added after 0.9.4; left out while
    /// there are none, so a board without any is written as before.
    #[serde(default, skip_serializing_if = "Records::is_empty")]
    pub shapes: Records<BoardShape>,
    /// Links between things on the board, oldest first.
    #[serde(default, skip_serializing_if = "Records::is_empty")]
    pub links: Records<BoardLink>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Something made from the floating capture panel, and where it went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRecord {
    pub id: String,
    pub kind: CaptureKind,
    pub project_id: String,
    /// What it was called when it was made. A session's own title wins once it has one.
    pub title: String,
    pub created_at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    /// Fields this version doesn't know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A project or group asked for by an id that isn't there.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotFound {
    #[error("Unknown project {0}")]
    Project(String),
    #[error("Unknown group {0}")]
    Group(String),
}

impl Default for State {
    /// The state before anything is loaded, and when nothing could be.
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            groups: Vec::new(),
            boards: IndexMap::new(),
            settings: AppSettings::default(),
            actions: None,
            voice_key: None,
            captures: None,
            servers: Vec::new(),
            server_boards: IndexMap::new(),
            extra: Map::new(),
        }
    }
}

impl Default for BoardState {
    /// A board no one has opened yet.
    fn default() -> Self {
        Self {
            nodes: IndexMap::new(),
            archive: Rect {
                x: 0.0,
                y: 0.0,
                w: DEFAULT_ARCHIVE_W,
                h: 320.0,
            },
            hidden: Vec::new(),
            sections: IndexMap::new(),
            notes: IndexMap::new(),
            views: IndexMap::new(),
            viewport: None,
            ink: None,
            shapes: Records::default(),
            links: Records::default(),
            extra: Map::new(),
        }
    }
}

impl State {
    /// Reads what state.json held as loadState does, taking what's usable:
    /// lists that aren't lists are empty, settings are normalized, and an
    /// empty voice key is none. Fails only when something it reads has a
    /// shape this version can't hold, which the TypeScript would have kept as
    /// it was.
    pub fn from_json(raw: Value) -> serde_json::Result<Self> {
        // Anything else has none of the fields.
        let Value::Object(mut raw) = raw else {
            return Ok(Self::default());
        };
        let projects = take_array(&mut raw, "projects")?.unwrap_or_default();
        let groups = take_array(&mut raw, "groups")?.unwrap_or_default();
        let boards = match raw.shift_remove("boards") {
            Some(boards @ Value::Object(_)) => serde_json::from_value(boards)?,
            _ => IndexMap::new(),
        };
        let settings = normalize_settings(raw.shift_remove("settings").as_ref().unwrap_or(&Value::Null));
        let actions = take_array(&mut raw, "actions")?;
        let voice_key = match raw.shift_remove("voiceKey") {
            Some(Value::String(key)) if !key.is_empty() => Some(key),
            _ => None,
        };
        let captures = take_array(&mut raw, "captures")?;
        let servers = take_array(&mut raw, "servers")?.unwrap_or_default();
        let server_boards = match raw.shift_remove("serverBoards") {
            Some(boards @ Value::Object(_)) => serde_json::from_value(boards)?,
            _ => IndexMap::new(),
        };
        Ok(Self {
            projects,
            groups,
            boards,
            settings,
            actions,
            voice_key,
            captures,
            servers,
            server_boards,
            extra: raw,
        })
    }

    pub fn project(&self, id: &str) -> Result<&Project, NotFound> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| NotFound::Project(id.into()))
    }

    pub fn project_mut(&mut self, id: &str) -> Result<&mut Project, NotFound> {
        self.projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| NotFound::Project(id.into()))
    }

    pub fn group(&self, id: &str) -> Result<&ProjectGroup, NotFound> {
        self.groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| NotFound::Group(id.into()))
    }

    pub fn group_mut(&mut self, id: &str) -> Result<&mut ProjectGroup, NotFound> {
        self.groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| NotFound::Group(id.into()))
    }

    /// A project's board, if it has one yet.
    pub fn board(&self, project_id: &str) -> Option<&BoardState> {
        self.boards.get(project_id)
    }

    /// A project's board, made empty if it has none, as getBoard does. That
    /// happens for any id, known project or not, and is saved with the rest.
    pub fn board_mut(&mut self, project_id: &str) -> &mut BoardState {
        self.boards.entry(project_id.to_owned()).or_default()
    }

    /// The project whose board has a view.
    pub fn project_id_for_view(&self, view_id: &str) -> Option<&str> {
        self.boards
            .iter()
            .find(|(_, board)| board.views.contains_key(view_id))
            .map(|(id, _)| id.as_str())
    }

    /// A server, by its id.
    pub fn server(&self, id: &str) -> Option<&ServerRecord> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// The server a project lives on, for a server project.
    pub fn project_server(&self, project_id: &str) -> Option<&str> {
        self.projects
            .iter()
            .find(|p| p.id == project_id)
            .and_then(|p| p.server.as_deref())
    }

    /// The project that owns a session card.
    pub fn project_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.boards
            .iter()
            .find(|(_, board)| board.nodes.contains_key(session_id))
            .map(|(id, _)| id.as_str())
    }
}

/// Settings as normalizeSettings reads them: defaults for what's missing, and
/// a value this version doesn't understand read as the default. The switches
/// that start on stay on unless they're `false`; the ones that start off stay
/// off unless they're `true`. Settings it doesn't know are kept, where the
/// TypeScript drops them, so a newer version's survive.
pub fn normalize_settings(raw: &Value) -> AppSettings {
    const KNOWN: [&str; 9] = [
        "theme",
        "autoUpdate",
        "notify",
        "dockBadge",
        "pinProjectRail",
        "focusFeature",
        "focusMode",
        "capture",
        "wheelZoom",
    ];
    let empty = Map::new();
    let raw = raw.as_object().unwrap_or(&empty);
    let unless_false = |key: &str| raw.get(key) != Some(&Value::Bool(false));
    let only_if_true = |key: &str| raw.get(key) == Some(&Value::Bool(true));
    AppSettings {
        theme: raw
            .get("theme")
            .and_then(Value::as_str)
            .and_then(ThemePreference::parse)
            .unwrap_or(ThemePreference::System),
        auto_update: unless_false("autoUpdate"),
        notify: unless_false("notify"),
        dock_badge: unless_false("dockBadge"),
        pin_project_rail: only_if_true("pinProjectRail"),
        focus_feature: only_if_true("focusFeature"),
        focus_mode: only_if_true("focusMode"),
        capture: unless_false("capture"),
        wheel_zoom: unless_false("wheelZoom"),
        extra: raw
            .iter()
            .filter(|(key, _)| !KNOWN.contains(&key.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

/// A top-level list, when it is one.
fn take_array<T: serde::de::DeserializeOwned>(
    raw: &mut Map<String, Value>,
    key: &str,
) -> serde_json::Result<Option<T>> {
    match raw.shift_remove(key) {
        Some(list @ Value::Array(_)) => serde_json::from_value(list).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_nothing_to_the_defaults() {
        for raw in [Value::Null, json!([]), json!("dark"), json!(5), json!({})] {
            assert_eq!(normalize_settings(&raw), AppSettings::default(), "{raw}");
        }
    }

    #[test]
    fn takes_only_the_themes_it_knows() {
        for (theme, expected) in [
            (json!("light"), ThemePreference::Light),
            (json!("dark"), ThemePreference::Dark),
            (json!("system"), ThemePreference::System),
            (json!("Dark"), ThemePreference::System),
            (json!(""), ThemePreference::System),
            (json!(1), ThemePreference::System),
        ] {
            assert_eq!(
                normalize_settings(&json!({ "theme": theme })).theme,
                expected,
                "{theme}"
            );
        }
    }

    #[test]
    fn turns_switches_off_only_with_false_and_on_only_with_true() {
        let all = |value: &Value| {
            json!({
                "autoUpdate": value, "notify": value, "dockBadge": value, "capture": value, "wheelZoom": value,
                "pinProjectRail": value, "focusFeature": value, "focusMode": value
            })
        };
        let off = normalize_settings(&all(&json!(false)));
        assert!(!off.auto_update && !off.notify && !off.dock_badge && !off.capture && !off.wheel_zoom);
        let on = normalize_settings(&all(&json!(true)));
        assert!(on.pin_project_rail && on.focus_feature && on.focus_mode);
        for value in [json!(0), json!(1), json!("false"), json!("true"), Value::Null] {
            assert_eq!(normalize_settings(&all(&value)), AppSettings::default(), "{value}");
        }
    }

    #[test]
    fn keeps_servers_and_their_boards_and_leaves_them_out_while_there_are_none() {
        let raw = json!({
            "projects": [{
                "id": "p1", "name": "Shop", "repoPath": "", "color": "blue", "createdAt": 1,
                "server": "s1", "later": true
            }],
            "servers": [{ "id": "s1", "url": "http://nodus:7420", "name": "nodus", "token": "djEw", "login": "sam" }],
            "serverBoards": { "p1": { "viewport": { "x": 1, "y": 2, "zoom": 0.5 }, "expanded": ["c1"] } }
        });
        let state = State::from_json(raw.clone()).unwrap();
        assert_eq!(state.project_server("p1"), Some("s1"));
        assert_eq!(state.server("s1").unwrap().extra["login"], "sam");
        assert_eq!(state.server_boards["p1"].expanded, ["c1"]);
        assert_eq!(state.projects[0].extra["later"], true);
        let written: Value = serde_json::to_value(&state).unwrap();
        let text = |value: &Value| nodus_domain::json::to_string(value).unwrap();
        for key in ["servers", "serverBoards"] {
            assert_eq!(text(&written[key]), text(&raw[key]), "{key}");
        }
        assert_eq!(written["projects"][0]["server"], "s1");
        let empty = serde_json::to_value(State::default()).unwrap();
        assert!(empty.get("servers").is_none() && empty.get("serverBoards").is_none());
    }

    #[test]
    fn reads_anything_but_an_object_as_empty_state() {
        for raw in [Value::Null, json!([]), json!("state"), json!(1)] {
            assert_eq!(State::from_json(raw).unwrap(), State::default());
        }
    }
}
