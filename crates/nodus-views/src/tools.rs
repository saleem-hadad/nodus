//! What the board tools do with views: `create_view`, `update_view`,
//! `get_view` and `list_views` (VIEW-102 to VIEW-115).

use nodus_board::geometry::{PartialSize, Size, VIEW_H, VIEW_W, clamp_view_size};
use nodus_domain::{BoardView, ViewEvent, js};
use serde_json::{Map, Value, json};

use crate::ViewError;
use crate::check::{
    MAX_DATA_CHARS, MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, check_html, check_json, clip, is_allowed,
    is_command_waiting, is_denied, js_number, normalize_commands, normalize_refresh,
};
use crate::runtime::{read_html, write_html};
use crate::views::{Views, set_title};

/// The names of the commands waiting for the user (VIEW-106).
fn waiting_names(view: &BoardView) -> Value {
    Value::Array(
        view.commands
            .iter()
            .flatten()
            .filter(|c| is_command_waiting(c))
            .map(|c| Value::String(c.name.clone()))
            .collect(),
    )
}

/// `clampViewSize({w: Number(width), h: Number(height)})` (VIEW-104).
fn tool_size(args: &Value) -> PartialSize {
    clamp_view_size(PartialSize {
        w: Some(js_number(args.get("width"))),
        h: Some(js_number(args.get("height"))),
    })
}

/// Long output cut to 4000 UTF-16 units, saying how much more there was (VIEW-113).
fn brief(s: &str) -> String {
    let len = js::utf16_len(s);
    if len > 4000 {
        format!("{}… ({} more characters)", js::slice_utf16(s, 0, 4000), len - 4000)
    } else {
        s.to_owned()
    }
}

/// `new Date(ms).toISOString()`.
fn iso(ms: f64) -> String {
    nodus_transcripts::date::to_iso_string(ms)
}

impl Views {
    /// The calling session's own view, when it's a view's brain (VIEW-109).
    fn own_view(&self, project_id: &str, self_id: &str) -> Result<BoardView, ViewError> {
        let view_of = self.inner.store.read(|state| {
            state
                .board(project_id)?
                .nodes
                .get(self_id)?
                .view_of
                .clone()
                .filter(|v| !v.is_empty())
        });
        let Some(view_of) = view_of else {
            return Err(ViewError::new("view_id is required: this session isn’t a view’s brain"));
        };
        self.find_view(project_id, Some(&Value::String(view_of)))
    }

    /// The view a tool means: the one it names, or the caller's own when it
    /// names none (VIEW-109, VIEW-113).
    fn tool_view(&self, project_id: &str, self_id: &str, view_id: Option<&Value>) -> Result<BoardView, ViewError> {
        match view_id {
            None => self.own_view(project_id, self_id),
            Some(id) => self.find_view(project_id, Some(id)),
        }
    }

    /// `create_view`: a session puts up a view of its own making, beside its
    /// card. Its brain starts when first needed (VIEW-102 to VIEW-107).
    pub async fn create_view_from_tool(
        &self,
        project_id: &str,
        self_id: &str,
        args: &Value,
    ) -> Result<Value, ViewError> {
        let title = clip(args.get("title"), MAX_TITLE_CHARS);
        if title.is_empty() {
            return Err(ViewError::new("A view needs a title"));
        }
        let html = check_html(args.get("html"))?;
        let data = args.get("data").cloned();
        check_json(data.as_ref(), MAX_DATA_CHARS, "The data")?;
        let commands = args.get("commands").map(|c| normalize_commands(c, None)).transpose()?;
        let refresh = normalize_refresh(args.get("refresh"))?;
        let size = tool_size(args);
        let place = self.place_view(
            project_id,
            self_id,
            Size {
                w: size.w.unwrap_or(VIEW_W),
                h: size.h.unwrap_or(VIEW_H),
            },
        );
        let now = self.now();
        let description = clip(args.get("description"), MAX_DESCRIPTION_CHARS);
        let view = BoardView {
            id: uuid::Uuid::new_v4().to_string(),
            description: if description.is_empty() {
                title.clone()
            } else {
                description
            },
            title,
            x: place.x,
            y: place.y,
            w: size.w,
            h: size.h,
            from: Some(self_id.to_owned()),
            commands: commands.filter(|c| !c.is_empty()),
            refresh: refresh.clone(),
            // VIEW-115: only `true` itself.
            starts_sessions: (args.get("start_sessions") == Some(&Value::Bool(true))).then_some(true),
            version: 1,
            created_at: now,
            updated_at: now,
            session_id: None,
            allowed_runs: None,
            denied_runs: None,
            extra: Map::new(),
        };
        write_html(&self.inner.user_data, &view.id, &html)?;
        self.with_runtime(&view.id, |rt| {
            rt.data = data;
            if refresh.is_some() {
                rt.refreshed_at = Some(now);
            }
        });
        self.save_runtime(&view.id);
        self.inner.store.update(|state| {
            state.board_mut(project_id).views.insert(view.id.clone(), view.clone());
        });
        self.board_changed(project_id);
        let page = self.check_page(&view.id, now).await;
        let waiting = waiting_names(&self.view(project_id, &view.id).unwrap_or(view.clone()));
        let mut out = Map::new();
        out.insert("view_id".into(), Value::String(view.id));
        out.insert("waiting_for_approval".into(), waiting);
        out.extend(page);
        Ok(Value::Object(out))
    }

    /// `update_view`: changes a view's page, data, title, commands or refresh
    /// schedule, checking everything before it changes anything (VIEW-109 to
    /// VIEW-112, VIEW-115).
    pub async fn update_view_from_tool(
        &self,
        project_id: &str,
        self_id: &str,
        args: &Value,
    ) -> Result<Value, ViewError> {
        let view = self.tool_view(project_id, self_id, args.get("view_id"))?;
        let html = args.get("html").map(|h| check_html(Some(h))).transpose()?;
        let data = args.get("data").cloned();
        check_json(data.as_ref(), MAX_DATA_CHARS, "The data")?;
        let commands = args
            .get("commands")
            .map(|c| normalize_commands(c, Some(&view)))
            .transpose()?;
        let refresh = args.get("refresh").map(|r| normalize_refresh(Some(r))).transpose()?;
        let title = clip(args.get("title"), MAX_TITLE_CHARS);
        let now = self.now();
        let size = tool_size(args);
        let description = args.get("description").map(|d| clip(Some(d), MAX_DESCRIPTION_CHARS));
        let start_sessions = args.get("start_sessions").and_then(Value::as_bool);

        if let Some(html) = &html {
            write_html(&self.inner.user_data, &view.id, html)?;
        }
        self.update_view(project_id, &view.id, |board, id| {
            if !title.is_empty() {
                set_title(board, id, &title);
            }
            let v = &mut board.views[id];
            if let Some(description) = description.filter(|d| !d.is_empty()) {
                v.description = description;
            }
            if let Some(commands) = commands {
                v.commands = (!commands.is_empty()).then_some(commands);
            }
            if let Some(refresh) = &refresh {
                v.refresh = refresh.clone();
            }
            if size.w.is_some() {
                v.w = size.w;
            }
            if size.h.is_some() {
                v.h = size.h;
            }
            match start_sessions {
                Some(true) => v.starts_sessions = Some(true),
                Some(false) => v.starts_sessions = None,
                None => {}
            }
            if html.is_some() {
                v.version += 1;
            }
            v.updated_at = now;
            Ok(())
        })?;
        self.with_runtime(&view.id, |rt| {
            if refresh.is_some() {
                rt.refreshed_at = Some(now);
            }
            if let Some(data) = &data {
                rt.data = Some(data.clone());
            }
        });
        // A page about to reload reads the data as it starts.
        if let Some(data) = &data
            && html.is_none()
        {
            self.notify(project_id, &view.id, ViewEvent::Data, data.clone());
        }
        self.save_runtime(&view.id);
        self.board_changed(project_id);
        let page = match html {
            Some(_) => self.check_page(&view.id, now).await,
            None => Map::new(),
        };
        let now_view = self.view(project_id, &view.id).unwrap_or(view);
        let mut out = Map::new();
        out.insert("view_id".into(), Value::String(now_view.id.clone()));
        out.insert("version".into(), Value::from(now_view.version));
        out.insert("waiting_for_approval".into(), waiting_names(&now_view));
        out.extend(page);
        Ok(Value::Object(out))
    }

    /// `get_view`: a view as it is now, for Claude to work on (VIEW-113).
    pub fn get_view_for_tool(
        &self,
        project_id: &str,
        self_id: &str,
        view_id: Option<&Value>,
    ) -> Result<Value, ViewError> {
        let view = self.tool_view(project_id, self_id, view_id)?;
        let (data, state, results, errors) = self.with_runtime(&view.id, |rt| {
            let skip = rt.errors.len().saturating_sub(10);
            (
                rt.data.clone(),
                rt.state.clone(),
                rt.results.clone(),
                rt.errors[skip..].to_vec(),
            )
        });
        let mut out = json!({
            "id": view.id,
            "title": view.title,
            "description": view.description,
            "html": read_html(&self.inner.user_data, &view.id),
        });
        if let Some(data) = data {
            out["data"] = data;
        }
        if let Some(state) = state {
            out["state"] = state;
        }
        let commands: Vec<Value> = view
            .commands
            .iter()
            .flatten()
            .map(|c| {
                let mut item = json!({ "name": c.name, "run": c.run });
                if let Some(every) = c.every {
                    item["every"] = Value::from(every);
                }
                item["allowed"] = Value::from(if is_allowed(c) {
                    "yes"
                } else if is_denied(c) {
                    "no, the user turned it down"
                } else {
                    "waiting for the user"
                });
                if let Some(Value::Object(last)) = results.get(&c.name) {
                    let mut last = last.clone();
                    for key in ["stdout", "stderr"] {
                        if let Some(Value::String(text)) = last.get(key) {
                            let cut = brief(text);
                            last.insert(key.into(), Value::String(cut));
                        }
                    }
                    item["last"] = Value::Object(last);
                }
                item
            })
            .collect();
        out["commands"] = Value::Array(commands);
        if let Some(refresh) = &view.refresh {
            out["refresh"] = serde_json::to_value(refresh).unwrap_or(Value::Null);
        }
        out["start_sessions"] = Value::Bool(view.starts_sessions == Some(true));
        out["errors"] = Value::Array(
            errors
                .iter()
                .map(|e| json!({ "message": e.message, "at": iso(e.at) }))
                .collect(),
        );
        Ok(out)
    }

    /// `list_views`: the board's views, oldest first, saying which are the
    /// caller's: it's their brain, or it put them up (VIEW-114).
    pub fn list_views_for_tool(&self, project_id: &str, self_id: &str) -> Value {
        let mut views: Vec<BoardView> = self.inner.store.read(|state| {
            state
                .board(project_id)
                .map(|b| b.views.values().cloned().collect())
                .unwrap_or_default()
        });
        views.sort_by(|a, b| {
            a.created_at
                .partial_cmp(&b.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Value::Array(
            views
                .iter()
                .map(|v| {
                    let mut item = json!({ "id": v.id, "title": v.title, "description": v.description });
                    if v.session_id.as_deref() == Some(self_id) || v.from.as_deref() == Some(self_id) {
                        item["yours"] = Value::Bool(true);
                    }
                    item
                })
                .collect(),
        )
    }
}
