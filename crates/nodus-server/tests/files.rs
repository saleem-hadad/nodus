//! A server's files over HTTP (SRV-022, SRV-163): `PUT /api/uploads` and
//! `GET /api/files/<id>`, against the real `nodus-server serve` on a
//! loopback port, with a data folder seeded with a board picture and a
//! view. Every server and host a test starts is stopped before it ends.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use nodus_wire::client::{download, upload};
use nodus_wire::{MAX_UPLOAD, Upload};
use serde_json::json;

const TOKEN: &str = "a-dev-token";
const VIEW: &str = "0b9c41aa-1d2e-4f3a-8b4c-5d6e7f8a9b0c";
const PICTURE: &str = "6f1c2d3e-4a5b-4c6d-8e7f-901a2b3c4d5e.png";

struct World {
    dir: tempfile::TempDir,
    server: Option<Child>,
    port: u16,
    /// Where the server keeps uploads: `--uploads`, else `<data>/uploads`.
    uploads: PathBuf,
}

impl World {
    /// A server whose data holds a project with a view, and a picture.
    fn new() -> Self {
        Self::start(false)
    }

    /// The same, with uploads kept apart from the data folder, as
    /// `install-server.sh` has it (`serve --uploads`).
    fn with_uploads_folder() -> Self {
        Self::start(true)
    }

    fn start(uploads_folder: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("nodus-files-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().to_path_buf();
        let data = root.join("data");
        let repo = root.join("repo");
        for folder in [&data, &repo, &root.join("home")] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let view = json!({
            "id": VIEW, "title": "Orders", "description": "orders", "x": 0, "y": 0,
            "version": 1, "createdAt": 1, "updatedAt": 1
        });
        let state = json!({
            "projects": [{ "id": "p1", "name": "Shop", "repoPath": repo, "color": "blue", "createdAt": 1 }],
            "boards": { "p1": {
                "nodes": {}, "archive": { "x": 0, "y": 0, "w": 100, "h": 100 }, "views": { VIEW: view }
            } }
        });
        std::fs::write(data.join("state.json"), state.to_string()).unwrap();
        let page = data.join("views").join(VIEW);
        std::fs::create_dir_all(&page).unwrap();
        std::fs::write(page.join("index.html"), "<h1>Orders today</h1>").unwrap();
        let images = data.join("board-images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join(PICTURE), b"\x89PNG picture").unwrap();

        // Port 0, and the one it got from what it prints: a port picked
        // here and let go could be taken by another test before it binds.
        let uploads = if uploads_folder {
            root.join("srv/uploads")
        } else {
            data.join("uploads")
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_nodus-server"));
        command
            .args(["serve", "--bind", "127.0.0.1:0", "--dev-token", TOKEN])
            .arg("--run-home")
            .arg(root.join("home"));
        if uploads_folder {
            command.arg("--uploads").arg(&uploads);
        }
        let mut child = command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", root.join("home"))
            .env("NODUS_USER_DATA", &data)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (listening, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(at) = line.find("listening http://") {
                    let _ = listening.send(line[at + "listening http://".len()..].trim().to_owned());
                }
            }
        });
        let address = heard.recv_timeout(Duration::from_secs(20)).expect("the server listens");
        let port = address
            .rsplit(':')
            .next()
            .and_then(|port| port.parse().ok())
            .expect("it listens on a port");
        Self {
            dir,
            server: Some(child),
            port,
            uploads,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn uploads(&self) -> PathBuf {
        self.uploads.clone()
    }

    /// A request as it is, and the status line the server answered.
    fn raw(&self, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer.lines().next().unwrap_or_default().to_owned()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
        let _ = Command::new("pkill")
            .args(["-f", &self.dir.path().to_string_lossy()])
            .status();
    }
}

/// SRV-022, SRV-163
#[tokio::test]
async fn keeps_an_upload_under_a_name_of_its_own_and_gives_it_back() {
    let world = World::new();
    let up: Upload = upload(&world.url(), TOKEN, "Screen Shot 10.00 ü.PNG", b"the bytes")
        .await
        .unwrap();
    // The server picks the name; the original is only a label.
    assert!(up.id.ends_with(".png") && up.id.len() == 36 + 4, "{}", up.id);
    assert_eq!(up.name, "Screen Shot 10.00 ü.PNG");
    assert_eq!(up.size, 9);
    let kept = world.uploads().join(&up.id);
    assert_eq!(PathBuf::from(&up.path), kept);
    assert_eq!(std::fs::read(&kept).unwrap(), b"the bytes");

    let back = download(&world.url(), TOKEN, &up.id).await.unwrap();
    assert_eq!((back.status, back.body.as_slice()), (200, &b"the bytes"[..]));
    assert_eq!(back.header("content-type"), Some("image/png"));
    assert_eq!(back.header("x-content-type-options"), Some("nosniff"));

    // A name with no extension worth keeping gets none.
    let plain = upload(&world.url(), TOKEN, "Makefile", b"all:").await.unwrap();
    assert_eq!(plain.id.len(), 36);
}

/// SRV-163: `serve --uploads` keeps uploads in that folder, never in the
/// data folder, which sessions run as another user can't read; and each is
/// readable by the folder's group (`install-server.sh`'s
/// `/srv/nodus/uploads`, group `nodus-run`), whatever the umask.
#[tokio::test]
async fn keeps_uploads_where_serve_uploads_says_readable_by_its_group() {
    use std::os::unix::fs::PermissionsExt as _;

    let world = World::with_uploads_folder();
    assert_eq!(world.uploads(), world.dir.path().join("srv/uploads"));
    let up: Upload = upload(&world.url(), TOKEN, "notes.md", b"# notes").await.unwrap();
    assert!(up.id.ends_with(".md") && up.id.len() == 36 + 3, "{}", up.id);
    let kept = world.uploads().join(&up.id);
    assert_eq!(PathBuf::from(&up.path), kept);
    assert_eq!(std::fs::read(&kept).unwrap(), b"# notes");
    assert!(!world.dir.path().join("data/uploads").exists());
    // Only the file itself is there: no part file left behind.
    let names: Vec<_> = std::fs::read_dir(world.uploads())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(&up.id)]);

    let mode = std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640, "{mode:o}");

    let back = download(&world.url(), TOKEN, &up.id).await.unwrap();
    assert_eq!((back.status, back.body.as_slice()), (200, &b"# notes"[..]));
}

/// SRV-022: the token, in the Authorization header only.
#[tokio::test]
async fn turns_away_a_missing_or_wrong_token() {
    let world = World::new();
    let wrong = upload(&world.url(), "not-the-token", "a.txt", b"x").await.unwrap_err();
    assert_eq!(wrong.to_string(), "The server refused the connection (401)");
    let refused = download(&world.url(), "not-the-token", PICTURE).await.unwrap();
    assert_eq!(refused.status, 401);
    let host = format!("Host: 127.0.0.1:{}", world.port);
    let none = world.raw(&format!(
        "GET /api/files/{PICTURE} HTTP/1.1\r\n{host}\r\nConnection: close\r\n\r\n"
    ));
    assert!(none.contains(" 401"), "{none}");
    let query = world.raw(&format!(
        "GET /api/files/{PICTURE}?token={TOKEN} HTTP/1.1\r\n{host}\r\nConnection: close\r\n\r\n"
    ));
    assert!(query.contains(" 401"), "{query}");
    assert!(!world.uploads().exists());
}

/// SRV-163: over 50 MB is refused, by what the request says or by what it sends.
#[tokio::test]
async fn refuses_an_upload_over_50_mb() {
    let world = World::new();
    let host = format!("Host: 127.0.0.1:{}", world.port);
    let said = world.raw(&format!(
        "PUT /api/uploads?name=big.bin HTTP/1.1\r\n{host}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_UPLOAD + 1
    ));
    assert!(said.contains(" 413"), "{said}");
    let big = vec![7u8; usize::try_from(MAX_UPLOAD).unwrap() + 1];
    let err = upload(&world.url(), TOKEN, "big.bin", &big).await.unwrap_err();
    assert!(
        err.to_string().contains("413") || err.to_string().contains("reset") || err.to_string().contains("pipe"),
        "{err}"
    );
    let kept = std::fs::read_dir(world.uploads()).map_or(0, Iterator::count);
    assert_eq!(kept, 0);
}

/// SRV-163: only ids the server issued, never a path.
#[tokio::test]
async fn serves_only_ids_it_issued() {
    let world = World::new();
    for id in [
        "../state.json",
        "%2e%2e%2fstate.json",
        "state.json",
        "6f1c2d3e-4a5b-4c6d-8e7f-000000000000.png",
        "6F1C2D3E-4A5B-4C6D-8E7F-901A2B3C4D5E.png",
    ] {
        let answer = download(&world.url(), TOKEN, id).await.unwrap();
        assert_eq!(answer.status, 404, "{id}");
    }
    let host = format!("Host: 127.0.0.1:{}", world.port);
    let dots = world.raw(&format!(
        "GET /api/files/../state.json HTTP/1.1\r\n{host}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    ));
    assert!(dots.contains(" 404"), "{dots}");
}

/// SRV-163: board pictures, and a view's page as its frame would get it.
#[tokio::test]
async fn serves_board_pictures_and_view_pages() {
    let world = World::new();
    let picture = download(&world.url(), TOKEN, PICTURE).await.unwrap();
    assert_eq!(
        (picture.status, picture.body.as_slice()),
        (200, &b"\x89PNG picture"[..])
    );

    let page = download(&world.url(), TOKEN, VIEW).await.unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(page.header("content-type"), Some("text/html; charset=utf-8"));
    assert!(page.header("content-security-policy").is_some());
    assert!(String::from_utf8_lossy(&page.body).contains("Orders today"));
}
