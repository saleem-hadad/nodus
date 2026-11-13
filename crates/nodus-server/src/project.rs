//! `nodus-server project add`: a folder on this machine, or a repository
//! cloned with this machine's own git into `<data>/projects/<slug>/main`,
//! added as a project through the running server. For development, until
//! projects come from the GitHub App's repositories (SRV-120), which
//! replaces it.

use std::io::Write;
use std::path::{Path, PathBuf};

use nodus_domain::Project;
use nodus_wire::Request;
use nodus_wire::client::Client;

use crate::data;
use crate::net::{self, Tailscale};

/// What `project add` was asked.
pub(crate) struct AddOptions {
    /// A folder, or a git URL (`https://…`, `ssh://…`, `git@host:org/repo`).
    pub target: String,
    /// The project's name: the folder's or the repository's when none.
    pub name: Option<String>,
    /// The server's address: where `serve` listens by default when none.
    pub server: Option<String>,
    pub token: String,
}

/// Adds the project and says so on standard output, or why not on
/// standard error; gives the exit code.
pub(crate) fn add(options: AddOptions) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => return failed(&err.to_string()),
    };
    match runtime.block_on(run(options)) {
        Ok(project) => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "Added {} ({}) at {}", project.name, project.id, project.repo_path);
            0
        }
        Err(err) => failed(&err),
    }
}

fn failed(err: &str) -> i32 {
    let _ = writeln!(std::io::stderr().lock(), "nodus-server: {err}");
    1
}

async fn run(options: AddOptions) -> Result<Project, String> {
    let address = match options.server {
        Some(address) => address,
        None => net::bind_address(None, Tailscale::detect().await.ip)?.to_string(),
    };
    // Before cloning, so a wrong token or a stopped server costs nothing.
    let (client, _events) = Client::connect(&address, &options.token)
        .await
        .map_err(|err| format!("Can't reach the server at {address}: {err}"))?;
    let target = options.target;
    let (folder, name) = if is_git_url(&target) && !Path::new(&target).exists() {
        let data = data::data_dir().ok_or("No data folder: set NODUS_USER_DATA or HOME")?;
        let name = repo_name(&target);
        let folder = clone(&target, &data.join("projects"), &slug(&name)).await?;
        (folder, options.name.or(Some(name)))
    } else {
        // The server resolves it from its own folder, not this one.
        let folder = std::path::absolute(&target).map_err(|err| format!("{target}: {err}"))?;
        (folder, options.name)
    };
    let added = client
        .call_as(&Request::ProjectsAdd {
            repo_path: folder.to_string_lossy().into_owned(),
            name,
        })
        .await
        .map_err(|err| err.message);
    client.close();
    added
}

/// Clones `url` into `<projects>/<slug>/main`, with a number after the slug
/// when that's taken. Git asks for credentials as it would in a terminal.
async fn clone(url: &str, projects: &Path, slug: &str) -> Result<PathBuf, String> {
    let mut dir = projects.join(slug);
    let mut number = 1;
    while dir.exists() {
        number += 1;
        dir = projects.join(format!("{slug}-{number}"));
    }
    std::fs::create_dir_all(&dir).map_err(|err| format!("Can't make {}: {err}", dir.display()))?;
    let main = dir.join("main");
    // `--` so a URL can't be read as an option.
    let cloned = tokio::process::Command::new("git")
        .arg("clone")
        .arg("--")
        .arg(url)
        .arg(&main)
        .status()
        .await;
    match cloned {
        Ok(status) if status.success() => Ok(main),
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(format!("git couldn't clone {url}"))
        }
        Err(err) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(format!("Can't run git: {err}"))
        }
    }
}

/// Whether `target` is a git URL rather than a folder: one with a scheme,
/// or scp's `user@host:path`.
fn is_git_url(target: &str) -> bool {
    if target.contains("://") {
        return true;
    }
    target
        .split_once(':')
        .is_some_and(|(host, _)| host.contains('@') && !host.contains('/'))
}

/// The repository's name in its URL: `nashra` for `git@github.com:org/nashra.git`.
fn repo_name(url: &str) -> String {
    let path = url.trim_end_matches('/');
    let last = path.rsplit(['/', ':']).next().unwrap_or(path);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        "project".to_owned()
    } else {
        name.to_owned()
    }
}

/// A folder name for a project: lower case letters, digits, `-`, `_` and `.`.
fn slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches(['-', '.']);
    if slug.is_empty() {
        "project".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_git_url_from_a_folder() {
        for url in [
            "https://github.com/saleem-hadad/nodus.git",
            "ssh://git@github.com/org/repo",
            "file:///tmp/repo.git",
            "git@github.com:org/repo.git",
        ] {
            assert!(is_git_url(url), "{url}");
        }
        for folder in ["/srv/repos/shop", "shop", "./a:b", "C:/repos/shop", "dir/user@host:x"] {
            assert!(!is_git_url(folder), "{folder}");
        }
    }

    #[test]
    fn names_a_project_after_its_repository() {
        assert_eq!(repo_name("git@github.com:org/Nashra.git"), "Nashra");
        assert_eq!(repo_name("https://github.com/org/nodus/"), "nodus");
        assert_eq!(repo_name("file:///tmp/x/shop.git"), "shop");
        assert_eq!(repo_name("https://example.com/.git"), "project");
        assert_eq!(slug("Nashra"), "nashra");
        assert_eq!(slug("My App!"), "my-app");
        assert_eq!(slug("../.."), "project");
    }

    #[tokio::test]
    async fn clones_beside_any_clone_of_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        std::fs::create_dir_all(projects.join("shop")).unwrap();
        // Nothing to clone: the folder it would have used goes again.
        let missing = dir.path().join("missing.git");
        let err = clone(&format!("file://{}", missing.display()), &projects, "shop")
            .await
            .unwrap_err();
        assert!(err.contains("couldn't clone"), "{err}");
        assert!(!projects.join("shop-2").exists());
        assert!(projects.join("shop").exists());
    }
}
