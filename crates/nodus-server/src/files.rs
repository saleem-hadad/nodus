//! The server's files (SRV-022, SRV-163): `PUT /api/uploads` keeps what an
//! app sends (a file dropped on a terminal, a picture added to a board)
//! under a name the server picks, in a folder its sessions can read, and
//! `GET /api/files/<id>` gives back an upload, a board picture or a view's
//! page. Both need a device's token (or the dev token, which only a server
//! on loopback takes), never a setup token, and only ids of the form the
//! server issues are ever looked up, so no request can name a path. Only
//! regular files the server wrote itself are given back: the uploads
//! folder is one its sessions can read (SRV-163), so a link there, or a
//! file in the place of one, is never followed or served.

use std::path::{Path, PathBuf};

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_LENGTH, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use nodus_wire::files::{file_id_ok, is_uuid, name_from_query, upload_ext};
use nodus_wire::{FILES_PATH, MAX_UPLOAD, UPLOADS_PATH, Upload};

use crate::auth::{SetupUse, Who};
use crate::net::bearer;
use crate::serve::Server;

type Reply = Response<Full<Bytes>>;

/// Whether a path is one of the files' routes.
pub(crate) fn handles(path: &str) -> bool {
    path == UPLOADS_PATH || path.starts_with(FILES_PATH)
}

/// A request on one of the files' routes, once its `Host` was checked:
/// the token first, in the Authorization header and nowhere else (SRV-022),
/// as `/api/ws` takes it. A setup token is for the wizard's first steps
/// only (SRV-060), so it gets 403 here.
pub(crate) async fn route(server: &Server, req: Request<Incoming>) -> Reply {
    match bearer(req.headers()).and_then(|token| server.auth.authenticate(token, SetupUse::Refuse)) {
        Some(Who::Dev | Who::Device { .. }) => {}
        Some(Who::Setup { .. }) => return status(StatusCode::FORBIDDEN),
        None => return status(StatusCode::UNAUTHORIZED),
    }
    let path = req.uri().path().to_owned();
    match (req.method(), path.strip_prefix(FILES_PATH)) {
        (&Method::PUT, None) => upload(server, req).await,
        (&Method::GET, Some(id)) => download(server, id).await,
        _ => status(StatusCode::NOT_FOUND),
    }
}

/// Where uploads are kept without `serve --uploads`: `<data>/uploads`,
/// which only the server's own user can reach. That does for development
/// and tests, where sessions run as that user too; an installed server's
/// sessions run as `nodus-run`, so `install-server.sh` passes a folder they
/// can read (SRV-163).
pub(crate) fn uploads_dir(data: &Path) -> PathBuf {
    data.join("uploads")
}

/// An upload's mode: its group may read it. In `/srv/nodus/uploads`
/// (2750, `nodus:nodus-run`) every file takes the folder's group, so the
/// sessions, run as `nodus-run`, can read what was dropped on their
/// terminal, and change nothing there; nobody else can. Set outright,
/// whatever the umask.
#[cfg(unix)]
const UPLOAD_MODE: u32 = 0o640;

/// `PUT /api/uploads?name=<name>`: the body kept as `<uuid>[.<ext>]`, the
/// name only a label. Over [`MAX_UPLOAD`] it's refused with 413, by its
/// Content-Length when it says, else as soon as the body passes it.
async fn upload(server: &Server, req: Request<Incoming>) -> Reply {
    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > MAX_UPLOAD) {
        return status(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let name = req.uri().query().and_then(name_from_query).unwrap_or_default();
    let mut body = req.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            return status(StatusCode::BAD_REQUEST);
        };
        if let Some(data) = frame.data_ref() {
            if bytes.len() as u64 + data.len() as u64 > MAX_UPLOAD {
                return status(StatusCode::PAYLOAD_TOO_LARGE);
            }
            bytes.extend_from_slice(data);
        }
    }
    let id = match upload_ext(&name) {
        Some(ext) => format!("{}.{ext}", uuid::Uuid::new_v4()),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let dir = &server.uploads;
    let file = dir.join(&id);
    let size = bytes.len() as u64;
    let kept = {
        let (dir, file) = (dir.clone(), file.clone());
        tokio::task::spawn_blocking(move || keep(&dir, &file, &bytes))
    };
    if !matches!(kept.await, Ok(Ok(()))) {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let upload = Upload {
        id,
        path: file.to_string_lossy().into_owned(),
        name,
        size,
    };
    let body = nodus_domain::json::to_string(&upload).unwrap_or_default();
    let mut reply = Response::new(Full::new(Bytes::from(body)));
    reply
        .headers_mut()
        .insert("Content-Type", HeaderValue::from_static("application/json"));
    reply
}

/// `GET /api/files/<id>`: an upload, else a board picture, else (for a bare
/// UUID) a view's page as its frame would get it; 404 for anything else.
async fn download(server: &Server, id: &str) -> Reply {
    if !file_id_ok(id) {
        return status(StatusCode::NOT_FOUND);
    }
    let images = nodus_core::board_images_dir(&server.data);
    for dir in [server.uploads.clone(), images] {
        let path = dir.join(id);
        if let Ok(Ok(bytes)) = tokio::task::spawn_blocking(move || read_own(&path)).await {
            let mut reply = Response::new(Full::new(Bytes::from(bytes)));
            let headers = reply.headers_mut();
            headers.insert("Content-Type", HeaderValue::from_static(content_type(id)));
            headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
            // An id is never reused for other bytes.
            headers.insert(
                "Cache-Control",
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            );
            return reply;
        }
    }
    if is_uuid(id) {
        let page = server.nodus.views().serve_page(&format!("nodus-view://{id}/"));
        if page.status != 404 {
            let mut reply = Response::new(Full::new(Bytes::from(page.body)));
            *reply.status_mut() = StatusCode::from_u16(page.status).unwrap_or(StatusCode::OK);
            for (name, value) in page.headers {
                if let Ok(value) = HeaderValue::from_str(&value) {
                    reply.headers_mut().insert(name, value);
                }
            }
            return reply;
        }
    }
    status(StatusCode::NOT_FOUND)
}

/// Writes an upload as `.<id>.part` beside it, a new file, never one
/// already there or a link in its place, with its mode set on the file
/// itself, then renames it into place.
fn keep(dir: &Path, file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir)?;
    let id = file.file_name().unwrap_or_default().to_string_lossy();
    let partial = dir.join(format!(".{id}.part"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(UPLOAD_MODE);
    }
    let mut out = options.open(&partial)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        out.set_permissions(std::fs::Permissions::from_mode(UPLOAD_MODE))?;
    }
    out.write_all(bytes)?;
    out.sync_all()?;
    drop(out);
    std::fs::rename(&partial, file)
}

/// A file the server wrote itself: a regular file it owns, opened without
/// following a link at its end or waiting on a pipe.
fn read_own(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || !owned_by_me(&meta) {
        return Err(std::io::Error::other("not a file the server wrote"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_UPLOAD).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(unix)]
fn owned_by_me(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    // SAFETY: geteuid has no preconditions and can't fail.
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn owned_by_me(_: &std::fs::Metadata) -> bool {
    true
}

/// A file's type by its extension.
fn content_type(id: &str) -> &'static str {
    match id.rsplit_once('.').map(|(_, ext)| ext) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn status(code: StatusCode) -> Reply {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = code;
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_files_by_their_extension() {
        assert_eq!(content_type("a.png"), "image/png");
        assert_eq!(content_type("a.jpeg"), "image/jpeg");
        assert_eq!(
            content_type("0b9c41aa-1d2e-4f3a-8b4c-5d6e7f8a9b0c"),
            "application/octet-stream"
        );
        assert_eq!(content_type("a.exe"), "application/octet-stream");
    }

    #[cfg(unix)]
    #[test]
    fn gives_back_only_a_file_it_wrote_itself() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("0b9c41aa-1d2e-4f3a-8b4c-5d6e7f8a9b0c.txt");
        keep(dir.path(), &file, b"hello").unwrap();
        assert_eq!(read_own(&file).unwrap(), b"hello");

        // SRV-163: a link in an upload's place, to the server's own secret
        // or anything else, and a pipe, give nothing.
        let secret = dir.path().join("github-app.pem");
        std::fs::write(&secret, "-----BEGIN-----").unwrap();
        let planted = dir.path().join("11111111-1111-1111-1111-111111111111.txt");
        std::os::unix::fs::symlink(&secret, &planted).unwrap();
        assert!(read_own(&planted).is_err());
        let pipe = dir.path().join("22222222-2222-2222-2222-222222222222");
        assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
        assert!(read_own(&pipe).is_err());
        // And a link where an upload is being written is never written through.
        let next = dir.path().join("33333333-3333-3333-3333-333333333333.txt");
        let target = dir.path().join("target");
        std::os::unix::fs::symlink(&target, dir.path().join(".33333333-3333-3333-3333-333333333333.txt.part")).unwrap();
        assert!(keep(dir.path(), &next, b"x").is_err());
        assert!(!target.exists());
    }
}
