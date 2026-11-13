//! What a server gives its core ([`nodus_core::Hosting`]): sessions' own
//! tokens (SRV-143), commit identities (SRV-082), and installation tokens
//! for one repository with only the permissions a job needs, cached for 50
//! minutes (SRV-080). And what projects come from: the repositories the
//! App's installations reach (SRV-120).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use nodus_core::{GitIdentity, Hosting, HostingFuture, TokenScope};
use nodus_domain::{FolderColor, Project, ProjectGithub};
use nodus_wire::GithubRepo;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::auth::Auth;
use crate::store;

/// How long a minted token is used before another is minted: GitHub's
/// last an hour (SRV-080).
const TOKEN_LIFE: Duration = Duration::from_secs(50 * 60);
/// The file holding the key sessions' tokens are made with, 0600.
const SESSION_KEY_FILE: &str = "session-key";

/// A minted token's installation, repository (lower case) and use.
type TokenKey = (u64, String, TokenScope);

/// The server's side of [`Hosting`].
pub(crate) struct ServerHosting {
    auth: Arc<Auth>,
    /// Makes each session's token (SRV-143), so a token holds across
    /// restarts and nobody without this key can work one out.
    session_key: Vec<u8>,
    /// `nodus` itself, which git runs as its credential helper (SRV-084).
    exe: PathBuf,
    /// Tokens minted, by installation, repository and what they're for.
    tokens: Mutex<HashMap<TokenKey, (String, Instant)>>,
    /// The App's bot, once looked up, by App id.
    bot: Mutex<Option<(u64, GitIdentity)>>,
}

impl ServerHosting {
    /// Reads the session key in `data`, making one the first time.
    pub(crate) fn open(data: &Path, auth: Arc<Auth>) -> Result<Self, String> {
        let file = data.join(SESSION_KEY_FILE);
        let key = match std::fs::read_to_string(&file) {
            Ok(text) if text.trim().len() >= 64 => text.trim().to_owned(),
            Ok(_) | Err(_) => {
                let key = store::random_hex(32);
                store::write_private(&file, &key)?;
                key
            }
        };
        let exe = std::env::current_exe().map_err(|err| format!("Can't find this program: {err}"))?;
        Ok(Self {
            auth,
            session_key: key.into_bytes(),
            exe,
            tokens: Mutex::default(),
            bot: Mutex::default(),
        })
    }

    /// `github:repos` (SRV-120): every repository every installation
    /// reaches, in the order GitHub lists them.
    pub(crate) async fn repos(&self) -> Result<Vec<GithubRepo>, String> {
        let (key, app_id) = self.auth.app_key()?;
        let installations = self.auth.installations().await?;
        let mut repos = Vec::new();
        for installation in installations {
            let token = self
                .auth
                .github
                .installation_token(&key, app_id, installation.id, None, &json!({ "metadata": "read" }))
                .await?;
            for repo in self.auth.github.installation_repos(&token).await? {
                repos.push(GithubRepo {
                    full_name: repo.full_name,
                    private: repo.private,
                    default_branch: repo.default_branch.unwrap_or_else(|| "main".into()),
                    installation_id: installation.id,
                });
            }
        }
        Ok(repos)
    }

    /// The repository to add for `projects:add-github`, as the App reaches
    /// it now (SRV-120).
    pub(crate) async fn repo(&self, full_name: &str) -> Result<GithubRepo, String> {
        let repos = self.repos().await?;
        repos
            .into_iter()
            .find(|repo| repo.full_name.eq_ignore_ascii_case(full_name))
            .ok_or_else(|| format!("The GitHub App can't reach {full_name}. Install it on that repository first."))
    }
}

/// HMAC-SHA256, hex.
fn hmac_hex(key: &[u8], message: &str) -> String {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.iter().map(|b| b ^ byte).collect::<Vec<u8>>();
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message.as_bytes())
        .finalize();
    let outer = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

/// A path as a shell reads it, in single quotes.
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hosting for ServerHosting {
    fn session_token(&self, session_id: &str) -> String {
        hmac_hex(&self.session_key, &format!("session:{session_id}"))
    }

    /// SRV-082: `<githubId>+<login>@users.noreply.github.com`, so GitHub
    /// shows the member's avatar on the commit.
    fn author(&self, login: &str) -> Option<GitIdentity> {
        let member = self.auth.member_by_login(login)?;
        let name = member
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| member.login.clone());
        Some(GitIdentity {
            name,
            email: format!("{}+{}@users.noreply.github.com", member.github_id, member.login),
        })
    }

    /// SRV-082: the App's bot, `<slug>[bot]`, with its user id from GitHub
    /// once it's known.
    fn committer(&self) -> HostingFuture<'_, Option<GitIdentity>> {
        Box::pin(async move {
            let app = self.auth.app()?;
            if let Some((app_id, bot)) = lock(&self.bot).clone()
                && app_id == app.app_id
            {
                return Some(bot);
            }
            let login = format!("{}[bot]", app.slug);
            let found = self.auth.github.user_by_login(&login).await.ok().flatten();
            let email = match &found {
                Some(user) => format!("{}+{login}@users.noreply.github.com", user.id),
                None => format!("{login}@users.noreply.github.com"),
            };
            let bot = GitIdentity { name: login, email };
            if found.is_some() {
                *lock(&self.bot) = Some((app.app_id, bot.clone()));
            }
            Some(bot)
        })
    }

    fn mint<'a>(&'a self, github: &'a ProjectGithub, scope: TokenScope) -> HostingFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let repo = github
                .full_name
                .rsplit('/')
                .next()
                .unwrap_or(&github.full_name)
                .to_owned();
            let cached = (github.installation_id, github.full_name.to_ascii_lowercase(), scope);
            if let Some((token, at)) = lock(&self.tokens).get(&cached)
                && at.elapsed() < TOKEN_LIFE
            {
                return Ok(token.clone());
            }
            let permissions = match scope {
                TokenScope::Git => json!({ "contents": "write" }),
                TokenScope::Gh => json!({ "contents": "write", "pull_requests": "write", "issues": "write" }),
            };
            let (key, app_id) = self.auth.app_key()?;
            let token = self
                .auth
                .github
                .installation_token(&key, app_id, github.installation_id, Some(&[&repo]), &permissions)
                .await?;
            lock(&self.tokens).insert(cached, (token.clone(), Instant::now()));
            Ok(token)
        })
    }

    fn github_url(&self) -> String {
        self.auth.github.web().to_owned()
    }

    fn credential_helper(&self) -> String {
        format!("!{} git-credential", shell_quote(&self.exe))
    }
}

/// A project's folder name: lower case letters, digits, `-`, `_` and `.`.
pub(crate) fn slug(name: &str) -> String {
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

/// `projects/<slug>` under the run home that no project has yet, with a
/// number after the slug when that's taken: its clone goes in `main`.
pub(crate) fn project_folder(run_home: &Path, name: &str) -> PathBuf {
    let projects = run_home.join("projects");
    let slug = slug(name);
    let mut dir = projects.join(&slug);
    let mut number = 1;
    while dir.exists() {
        number += 1;
        dir = projects.join(format!("{slug}-{number}"));
    }
    dir.join("main")
}

/// A colour for a repository's project, the same every time (SRV-120).
pub(crate) fn repo_color(full_name: &str) -> FolderColor {
    let sum = full_name
        .bytes()
        .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(usize::from(b)));
    FolderColor::ALL[sum % FolderColor::ALL.len()]
}

/// `projects:add-github` (SRV-120): the repository as the App reaches it
/// now, cloned through the session host into the run home's `projects/`,
/// and added.
pub(crate) async fn add_github(
    server: &crate::serve::Server,
    full_name: &str,
    name: Option<String>,
    color: Option<FolderColor>,
) -> Result<Project, String> {
    let repo = server.hosting.repo(full_name).await?;
    let name = name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| repo.full_name.rsplit('/').next().unwrap_or(&repo.full_name).to_owned());
    let folder = project_folder(&server.run_home, &name);
    let github = ProjectGithub {
        full_name: repo.full_name.clone(),
        default_branch: repo.default_branch.clone(),
        installation_id: repo.installation_id,
    };
    let color = color.unwrap_or_else(|| repo_color(&repo.full_name));
    server
        .nodus
        .add_github_project(github, &folder, &name, Some(color))
        .await
        .map_err(|err| err.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_as_hmac_sha256_does() {
        // RFC 4231, test case 2.
        assert_eq!(
            hmac_hex(b"Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let long = [0xaa_u8; 131];
        // RFC 4231, test case 6: a key longer than a block.
        assert_eq!(
            hmac_hex(&long, "Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn names_a_projects_folder_after_its_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(slug("Nashra"), "nashra");
        assert_eq!(slug("My App!"), "my-app");
        assert_eq!(slug("../.."), "project");
        let first = project_folder(dir.path(), "Shop");
        assert_eq!(first, dir.path().join("projects/shop/main"));
        std::fs::create_dir_all(&first).unwrap();
        assert_eq!(
            project_folder(dir.path(), "shop"),
            dir.path().join("projects/shop-2/main")
        );
    }

    #[test]
    fn quotes_the_helper_for_git() {
        assert_eq!(shell_quote(Path::new("/opt/nodus/bin/nodus")), "'/opt/nodus/bin/nodus'");
        assert_eq!(shell_quote(Path::new("/tmp/it's")), r"'/tmp/it'\''s'");
        assert_eq!(repo_color("acme/shop"), repo_color("acme/shop"));
    }
}
