//! What the core asks of the OS before nodus-platform has it.

use std::path::Path;

/// `shell.showItemInFolder(path)`: Finder or File Explorer with the item
/// selected. Moves to `nodus_platform` (the platform track's open/reveal) once
/// it's there.
pub(crate) fn show_item_in_folder(path: &Path) {
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    let opened = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let opened = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    match opened {
        // Reaped off the caller's thread, so it never lingers as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!(path = %path.display(), %err, "couldn't reveal the folder"),
    }
}
