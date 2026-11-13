//! Each call a server answers, made on the core's API as the window makes
//! it (SRV-024): the answer is the serde form of what the core gives, and a
//! failure the channel and message of its `CommandError`, as the window
//! would have shown them.

use nodus_core::{CommandError, CommandResult, Nodus};
use nodus_domain::Event;
use nodus_wire::{CallError, Request};
use serde::Serialize;
use serde_json::Value;

/// Answers one call, made by `member` (their GitHub login; none for a
/// development token): what they add on a board records them as its
/// `createdBy` (SRV-183). `watch` and `session:close` change only the
/// connection, which sees to them (SRV-026), so here they're done at once.
pub(crate) async fn answer(nodus: Nodus, member: Option<String>, request: Request) -> Result<Value, CallError> {
    let channel = request.name();
    let by = nodus.by(member.as_deref());
    match request {
        Request::ProjectsList {} => give(channel, nodus.projects().list().await),
        Request::ProjectsAdd { repo_path, name } => {
            let added = nodus.projects().add_project(&repo_path, name.as_deref());
            projects_changed(&nodus, &added);
            done(channel, added.map(|(project, _)| project))
        }
        Request::ProjectsUpdate { id, patch } => {
            let updated = nodus.projects().update(&id, patch).await;
            projects_changed(&nodus, &updated);
            done(channel, updated)
        }
        Request::ProjectsRemove { id } => {
            nodus.projects().remove(&id).await;
            projects_changed(&nodus, &Ok(()));
            DONE
        }
        Request::ProjectsStats {} => give(channel, nodus.projects().stats().await),

        Request::GroupsList {} => give(channel, nodus.groups().list().await),
        Request::GroupsCreate { name } => {
            let group = nodus.groups().create(&name).await;
            projects_changed(&nodus, &Ok(()));
            give(channel, group)
        }
        Request::GroupsUpdate { id, patch } => {
            let updated = nodus.groups().update(&id, patch).await;
            projects_changed(&nodus, &updated);
            done(channel, updated)
        }
        Request::GroupsRemove { id } => {
            nodus.groups().remove(&id).await;
            projects_changed(&nodus, &Ok(()));
            DONE
        }

        Request::BoardLoad { project_id } => done(channel, nodus.board().load(&project_id).await),
        Request::BoardSaveNodes { project_id, patches } => {
            done(channel, nodus.board().save_nodes(&project_id, &patches).await)
        }
        Request::BoardSaveArchive { project_id, rect } => {
            nodus.board().save_archive(&project_id, rect).await;
            DONE
        }
        Request::BoardSaveViewport { project_id, viewport } => {
            nodus.board().save_viewport(&project_id, viewport).await;
            DONE
        }
        Request::BoardRemoveNode { project_id, session_id } => {
            nodus.board().remove_node(&project_id, &session_id).await;
            DONE
        }
        Request::BoardSaveSection { project_id, section } => {
            by.board().save_section(&project_id, &section).await;
            DONE
        }
        Request::BoardRemoveSection { project_id, section_id } => {
            nodus.board().remove_section(&project_id, &section_id).await;
            DONE
        }
        Request::BoardNameSection {
            project_id,
            session_ids,
        } => done(channel, nodus.board().name_section(&project_id, &session_ids).await),
        Request::BoardSaveNote { project_id, note } => {
            by.board().save_note(&project_id, &note).await;
            DONE
        }
        Request::BoardRemoveNotes { project_id, note_ids } => {
            nodus.board().remove_notes(&project_id, &note_ids).await;
            DONE
        }
        Request::BoardSaveInk { project_id, strokes } => {
            by.board().save_ink(&project_id, &strokes).await;
            DONE
        }
        Request::BoardSaveShapes { project_id, shapes } => {
            by.board().save_shapes(&project_id, &shapes).await;
            DONE
        }
        Request::BoardRemoveShapes { project_id, ids } => {
            nodus.board().remove_shapes(&project_id, &ids).await;
            DONE
        }
        Request::BoardSaveLinks { project_id, links } => {
            by.board().save_links(&project_id, &links).await;
            DONE
        }
        Request::BoardRemoveLinks { project_id, ids } => {
            nodus.board().remove_links(&project_id, &ids).await;
            DONE
        }

        Request::SessionCreate { project_id, opts } => done(channel, by.sessions().create(&project_id, &opts).await),
        Request::SessionOpen {
            project_id,
            session_id,
            cols,
            rows,
        } => done(
            channel,
            nodus.sessions().open(&project_id, &session_id, cols, rows).await,
        ),
        Request::SessionWrite { session_id, data } => {
            nodus.sessions().write(&session_id, &data);
            DONE
        }
        Request::SessionSend {
            project_id,
            session_id,
            text,
        } => done(channel, nodus.sessions().send(&project_id, &session_id, &text).await),
        Request::SessionResize { session_id, cols, rows } => {
            nodus.sessions().resize(&session_id, cols, rows);
            DONE
        }
        Request::SessionKill { session_id } => {
            nodus.sessions().kill(&session_id).await;
            DONE
        }
        Request::SessionStop { session_ids } => {
            nodus.sessions().stop(&session_ids).await;
            DONE
        }
        Request::SessionTranscript { project_id, session_id } => {
            done(channel, nodus.sessions().transcript(&project_id, &session_id).await)
        }
        Request::SessionSummarize {
            project_id,
            session_ids,
            request_id,
            focus,
        } => done(
            channel,
            nodus
                .sessions()
                .summarize(&project_id, &session_ids, &request_id, focus.as_deref())
                .await,
        ),
        Request::SessionSearch { project_id, query } => {
            done(channel, nodus.sessions().search(&project_id, &query).await)
        }
        Request::SessionAgents { project_id, session_id } => {
            done(channel, nodus.sessions().agents(&project_id, &session_id).await)
        }
        Request::SessionAgentTranscript {
            project_id,
            session_id,
            agent_id,
        } => done(
            channel,
            nodus
                .sessions()
                .agent_transcript(&project_id, &session_id, &agent_id)
                .await,
        ),

        Request::ViewsCreate { project_id, opts } => done(channel, by.views().create(&project_id, &opts).await),
        Request::ViewsSave { project_id, patch } => done(channel, nodus.views().save(&project_id, &patch).await),
        Request::ViewsRemove { project_id, view_id } => {
            done(channel, nodus.views().remove(&project_id, &view_id).await)
        }
        Request::ViewsCall {
            project_id,
            view_id,
            method,
            args,
        } => done(channel, nodus.views().call(&project_id, &view_id, &method, &args).await),
        Request::ViewsReport {
            project_id,
            view_id,
            report,
        } => {
            nodus.views().report(&project_id, &view_id, &report);
            DONE
        }
        Request::ViewsApprove {
            project_id,
            view_id,
            name,
            allow,
        } => done(
            channel,
            nodus.views().approve(&project_id, &view_id, &name, allow).await,
        ),
        Request::ViewsRefresh { project_id, view_id } => {
            done(channel, nodus.views().refresh(&project_id, &view_id).await)
        }
        Request::ViewsAsk {
            project_id,
            view_id,
            prompt,
        } => done(channel, nodus.views().ask(&project_id, &view_id, &prompt).await),
        Request::ViewsBrain { project_id, view_id } => done(channel, nodus.views().brain(&project_id, &view_id).await),

        // SRV-128: a card's own worktree when it's named.
        Request::GitStatus { project_id, session_id } => {
            done(channel, nodus.git_for(session_id.as_deref()).status(&project_id).await)
        }
        Request::GitDiff {
            project_id,
            file,
            session_id,
        } => done(
            channel,
            nodus.git_for(session_id.as_deref()).diff(&project_id, &file).await,
        ),
        Request::GitLog {
            project_id,
            limit,
            session_id,
        } => done(
            channel,
            nodus.git_for(session_id.as_deref()).log(&project_id, limit).await,
        ),

        Request::ActionsGlobal {} => give(channel, nodus.actions().global().await),
        Request::ActionsSave { project_id, action } => done(channel, nodus.actions().save(&project_id, &action).await),
        Request::ActionsRemove { project_id, action_id } => {
            done(channel, nodus.actions().remove(&project_id, &action_id).await)
        }
        Request::ActionsDraft {
            project_id,
            description,
            global,
        } => done(channel, nodus.actions().draft(&project_id, &description, global).await),

        Request::Watch { .. } | Request::SessionClose { .. } => DONE,

        // The server's own, answered by calls.rs with who's asking.
        Request::SetupStatus {}
        | Request::SetupName { .. }
        | Request::SetupDone { .. }
        | Request::MembersList {}
        | Request::MembersInvite { .. }
        | Request::MembersRemove { .. }
        | Request::DevicesList {}
        | Request::DevicesRevoke { .. }
        | Request::AuditList {}
        | Request::ServerInfo {}
        | Request::GithubInstallations {}
        | Request::GithubReplaceKey { .. }
        | Request::GithubRepos {}
        | Request::ProjectsAddGithub { .. } => Err(CallError::new(channel, nodus_wire::NOT_AVAILABLE)),
    }
}

/// What a command with nothing to give answers: `null`.
const DONE: Result<Value, CallError> = Ok(Value::Null);

/// A command's result as an answer.
pub(crate) fn done<T: Serialize>(channel: &str, result: CommandResult<T>) -> Result<Value, CallError> {
    match result {
        Ok(value) => give(channel, value),
        Err(err) => Err(CallError::new(err.channel, err.message)),
    }
}

/// What a command that can't fail gave, as an answer.
fn give<T: Serialize>(channel: &str, value: T) -> Result<Value, CallError> {
    serde_json::to_value(value).map_err(|err| CallError::new(channel, err.to_string()))
}

/// The projects or groups changed: every connected app hears of it, since
/// the one that changed them isn't the only one showing them.
fn projects_changed<T>(nodus: &Nodus, result: &Result<T, CommandError>) {
    if result.is_ok() {
        nodus.emit(Event::ProjectsChanged);
    }
}
