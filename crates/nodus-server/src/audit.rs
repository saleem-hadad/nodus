//! `audit.jsonl` (SRV-210): one JSON line per event, appended and never
//! rewritten, readable by `nodus` alone (0600).
//!
//! The events so far: sign-ins and refused sign-ins, devices added and
//! revoked, members invited and removed, setup claimed, the server named,
//! the GitHub App made, its key replaced and its installations changed.
//! Later slices add Claude accounts, updates and approvals.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use nodus_wire::{AuditDevice, AuditLine, AuditMember};
use serde_json::Value;

use crate::store::now_ms;

pub(crate) const AUDIT_FILE: &str = "audit.jsonl";
/// How many lines `audit:list` gives.
pub(crate) const LISTED: usize = 200;

pub(crate) struct Audit {
    file: PathBuf,
    /// One line at a time, whole.
    writing: Mutex<()>,
}

impl Audit {
    pub(crate) fn new(data: &Path) -> Self {
        Self {
            file: data.join(AUDIT_FILE),
            writing: Mutex::new(()),
        }
    }

    /// Appends a line. A log that can't be written doesn't stop what it
    /// records; it's said on standard error.
    pub(crate) fn record(&self, event: &str, member: Option<AuditMember>, device: Option<AuditDevice>, details: Value) {
        let line = AuditLine {
            time: now_ms(),
            event: event.to_owned(),
            member,
            device,
            details,
        };
        let Ok(mut text) = nodus_domain::json::to_string(&line) else {
            return;
        };
        text.push('\n');
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(err) = append(&self.file, &text) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "nodus-server: can't write {}: {err}",
                self.file.display()
            );
        }
    }

    /// The last [`LISTED`] lines, oldest first. Lines that aren't an
    /// [`AuditLine`] are skipped.
    pub(crate) fn last(&self) -> Vec<AuditLine> {
        let text = tail(&self.file, LISTED).unwrap_or_default();
        text.lines()
            .filter_map(|line| nodus_domain::json::from_str(line).ok())
            .collect()
    }
}

/// Appends in one write to a file opened for appending only, made 0600.
fn append(file: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(file)?.write_all(text.as_bytes())
}

/// The file's last `lines` whole lines, read from its end.
fn tail(file: &Path, lines: usize) -> std::io::Result<String> {
    let mut file = std::fs::File::open(file)?;
    let len = file.metadata()?.len();
    let mut window: u64 = 64 * 1024;
    loop {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        // The first line is cut unless the window reaches the start.
        let whole = if start == 0 {
            &text[..]
        } else {
            text.split_once('\n').map_or("", |(_, rest)| rest)
        };
        let count = whole.lines().count();
        if count >= lines || start == 0 {
            let skip = count.saturating_sub(lines);
            return Ok(whole.lines().skip(skip).collect::<Vec<_>>().join("\n"));
        }
        window *= 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appends_lines_and_lists_the_last_ones() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Audit::new(dir.path());
        assert!(audit.last().is_empty());
        let who = AuditMember {
            github_id: 1,
            login: "octo".into(),
        };
        for i in 0..250 {
            audit.record("member.invited", Some(who.clone()), None, json!({ "i": i }));
        }
        let last = audit.last();
        assert_eq!(last.len(), LISTED);
        assert_eq!(last[0].details["i"], 50);
        assert_eq!(last[199].details["i"], 249);
        assert_eq!(last[199].member.as_ref(), Some(&who));
        let text = std::fs::read_to_string(dir.path().join(AUDIT_FILE)).unwrap();
        assert_eq!(text.lines().count(), 250);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(AUDIT_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn reads_back_far_enough_for_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Audit::new(dir.path());
        let long = "x".repeat(2000);
        for i in 0..100 {
            audit.record("github.app-created", None, None, json!({ "i": i, "pad": long }));
        }
        let last = audit.last();
        assert_eq!(last.len(), 100);
        assert_eq!(last[0].details["i"], 0);
    }
}
