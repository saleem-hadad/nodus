//! The calls the server answers itself rather than its core: setup,
//! members, devices, the audit log, the server and its GitHub App
//! (SRV-040…063, SRV-210). Every member may make every one (SRV-040).

use std::sync::Arc;

use nodus_wire::{CallError, Request};
use serde::Serialize;
use serde_json::Value;

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
        Request::MembersRemove { github_id } => auth.remove_member(&who, github_id).map(|()| Value::Null),
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
        _ => Err(nodus_wire::NOT_AVAILABLE.to_owned()),
    };
    answered.map_err(|message| CallError::new(channel, message))
}

fn give<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|err| err.to_string())
}
