//! The newest commits on HEAD (`gitLog`, GIT-011…GIT-013).

use std::path::Path;

use nodus_domain::GitCommit;

use crate::runner::{DEFAULT_MAX_BUFFER, Git, RunError};

/// How many commits the branch chip's popover lists (GIT-013).
pub const DEFAULT_LOG_LIMIT: u32 = 5;

/// `git:log`'s count from the window: a whole number kept to 1–50, else 5
/// (GIT-013). `None` is a missing or non-numeric argument.
pub fn clamp_limit(limit: Option<f64>) -> u32 {
    match limit {
        // Number.isInteger: finite and whole.
        Some(n) if n.is_finite() && n.fract() == 0.0 => n.clamp(1.0, 50.0) as u32,
        _ => DEFAULT_LOG_LIMIT,
    }
}

/// GIT-011: fields are split by 0x1f and commits by NUL, neither of which can
/// be in a subject or a name.
pub fn log_args(limit: u32) -> Vec<String> {
    [
        "log".to_owned(),
        format!("--max-count={limit}"),
        "--no-show-signature".to_owned(),
        "--format=%H%x1f%an%x1f%at%x1f%s".to_owned(),
        "-z".to_owned(),
        "HEAD".to_owned(),
        "--".to_owned(),
    ]
    .into()
}

impl Git {
    /// The newest commits on HEAD, newest first; empty before the first
    /// commit or outside a repository (GIT-012).
    pub async fn log(&self, repo_path: &Path, limit: u32) -> Result<Vec<GitCommit>, RunError> {
        let args = log_args(limit);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let res = self.run(repo_path, &args, DEFAULT_MAX_BUFFER).await?;
        if res.code != 0 {
            return Ok(Vec::new());
        }
        Ok(parse_log(&res.stdout))
    }
}

/// `Number(text)`: blank is 0, anything that isn't a number is NaN.
fn js_number(text: &str) -> f64 {
    let text = nodus_domain::js::trim(text);
    if text.is_empty() {
        return 0.0;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// GIT-012.
pub fn parse_log(stdout: &str) -> Vec<GitCommit> {
    stdout
        .split('\0')
        .filter(|rec| !rec.is_empty())
        .map(|rec| {
            let mut fields = rec.split('\x1f');
            let mut next = || fields.next().unwrap_or_default().to_owned();
            let (oid, author, at, subject) = (next(), next(), next(), next());
            GitCommit {
                oid,
                subject,
                author,
                time: js_number(&at) * 1000.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_count_small_and_whole() {
        assert_eq!(clamp_limit(None), 5);
        assert_eq!(clamp_limit(Some(2.0)), 2);
        assert_eq!(clamp_limit(Some(0.0)), 1);
        assert_eq!(clamp_limit(Some(-4.0)), 1);
        assert_eq!(clamp_limit(Some(500.0)), 50);
        assert_eq!(clamp_limit(Some(2.5)), 5);
        assert_eq!(clamp_limit(Some(f64::NAN)), 5);
        assert_eq!(clamp_limit(Some(f64::INFINITY)), 5);
    }

    #[test]
    fn reads_commits() {
        assert_eq!(
            log_args(5),
            [
                "log",
                "--max-count=5",
                "--no-show-signature",
                "--format=%H%x1f%an%x1f%at%x1f%s",
                "-z",
                "HEAD",
                "--"
            ]
        );
        let out = "aaa\x1fTest\x1f1700000000\x1fFirst: tabs\tand | pipes\0bbb\x1fOther\x1f1700003600\x1fSecond\0";
        let log = parse_log(out);
        assert_eq!(log.len(), 2);
        assert_eq!(
            log[0],
            GitCommit {
                oid: "aaa".into(),
                subject: "First: tabs\tand | pipes".into(),
                author: "Test".into(),
                time: 1_700_000_000_000.0
            }
        );
        assert_eq!(log[1].time, 1_700_003_600_000.0);
    }
}
