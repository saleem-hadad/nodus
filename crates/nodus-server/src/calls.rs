//! The calls the server answers itself rather than its core: setup,
//! members, devices, the audit log, the server and its GitHub App, and its
//! pool of Claude accounts (SRV-040…063, SRV-100…105, SRV-210). Every
//! member may make every one (SRV-040).

use std::sync::Arc;

use nodus_wire::{CallError, Request};
use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::Who;
use crate::serve::Server;

/// Answers one of the server's own calls ([`Request::is_server_call`]).
pub(crate) async fn answer(server: Arc<Server>, who: Who, request: Request) -> Result<Value, CallError> {
    let channel = request.name();
    let auth = &server.auth;
    let answered = match request {
        Request::SetupStatus {} => give(auth.setup_status()),
        Request::SetupName { name } => auth.set_name(&who, &name).and_then(give),
        Request::SetupDone { step } => auth.setup_done(step).and_then(give),
        Request::MembersList {} => give(auth.members()),
        Request::MembersInvite { login } => auth.invite(&who, &login).await.and_then(give),
        Request::MembersRemove {
            github_id,
            remove_accounts,
        } => auth.remove_member(&who, github_id).map(|()| {
            // SRV-040: their Claude accounts too, when asked.
            if remove_accounts {
                server.hosting.pool.remove_added_by(&who, github_id);
            }
            Value::Null
        }),
        Request::DevicesList {} => give(auth.devices(&who)),
        Request::DevicesRevoke { id } => auth.revoke(&who, &id).map(|()| Value::Null),
        Request::AuditList {} => give(auth.audit.last()),
        Request::ServerInfo {} => give(auth.info().await),
        Request::GithubInstallations {} => auth.installations().await.and_then(give),
        Request::GithubReplaceKey { pem } => auth.replace_key(&who, &pem).await.and_then(give),
        Request::GithubRepos {} => server.hosting.repos().await.and_then(give),
        Request::ProjectsAddGithub { full_name, name, color } => {
            crate::hosting::add_github(&server, &full_name, name, color)
                .await
                .and_then(give)
        }
        // SRV-100, SRV-105: no answer holds a token.
        Request::AccountsList {} => give(server.hosting.pool.list(&server.nodus)),
        Request::AccountsAdd { label, token } => server
            .hosting
            .pool
            .add(&server.nodus, &who, &label, &token)
            .await
            .and_then(give),
        Request::AccountsRename { id, label } => server
            .hosting
            .pool
            .rename(&server.nodus, &who, &id, &label)
            .and_then(give),
        Request::AccountsRemove { id } => server.hosting.pool.remove(&who, &id).map(|()| Value::Null),
        _ => Err(nodus_wire::NOT_AVAILABLE.to_owned()),
    };
    answered.map_err(|message| CallError::new(channel, message))
}

/// `views:approve` (SRV-145): any member may allow or turn down a view's
/// command. The command records who allowed it (`approvedBy`), and the
/// audit log has a line for each (SRV-210).
pub(crate) async fn approve_view(server: Arc<Server>, who: Who, request: Request) -> Result<Value, CallError> {
    let channel = request.name();
    let Request::ViewsApprove {
        project_id,
        view_id,
        name,
        allow,
    } = request
    else {
        return Err(CallError::new(channel, nodus_wire::NOT_AVAILABLE));
    };
    let member = server.auth.login_of(&who);
    let approved = server
        .nodus
        .by(member.as_deref())
        .views()
        .approve(&project_id, &view_id, &name, allow)
        .await;
    let run = approved
        .as_ref()
        .ok()
        .and_then(|()| server.nodus.views().command_line(&project_id, &view_id, &name));
    approved.map_err(|err| CallError::new(err.channel, err.message))?;
    server.auth.record(
        &who,
        if allow {
            "view.command-approved"
        } else {
            "view.command-denied"
        },
        json!({ "projectId": project_id, "viewId": view_id, "name": name, "run": run }),
    );
    Ok(Value::Null)
}

fn give<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|err| err.to_string())
}
