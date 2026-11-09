#![allow(clippy::print_stdout)]
//! On Windows, gives `Nodus.exe` the icon and version strings 0.9.4's had
//! (REL-028): icon index 0 from `build/icon-win.png` (the uninstall entry's
//! `DisplayIcon` and the shortcuts point at it), FileDescription and
//! ProductName `Nodus`, CompanyName `Saleem Hadad`, LegalCopyright
//! `Copyright © <build year> Saleem Hadad`, FileVersion `<v>` and
//! ProductVersion `<v>.0`, as electron-builder's resource edit wrote them.
//! The manifest (asInvoker, per-monitor DPI, common controls 6) is GPUI's,
//! which its `windows-manifest` feature links into every binary using it.
//!
//! Elsewhere, and when the Windows resource compiler isn't there (checking
//! the Windows build from a Mac), it does nothing.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let icon = manifest_dir.join("../../packaging/windows/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let part = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let numbers = format!(
        "{},{},{},0",
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH")
    );
    let icon = icon.canonicalize().unwrap_or(icon);
    let icon = icon.to_string_lossy().replace('\\', "/");

    let mut rc = String::new();
    rc.push_str("#pragma code_page(65001)\n");
    let _ = writeln!(rc, "1 ICON \"{icon}\"");
    let _ = writeln!(rc, "1 VERSIONINFO");
    let _ = writeln!(rc, "FILEVERSION {numbers}");
    let _ = writeln!(rc, "PRODUCTVERSION {numbers}");
    rc.push_str("FILEFLAGSMASK 0x3fL\nFILEFLAGS 0x0L\nFILEOS 0x40004L\nFILETYPE 0x1L\nFILESUBTYPE 0x0L\n");
    rc.push_str("BEGIN\n  BLOCK \"StringFileInfo\"\n  BEGIN\n    BLOCK \"040904b0\"\n    BEGIN\n");
    for (key, value) in [
        ("CompanyName", "Saleem Hadad".to_owned()),
        ("FileDescription", "Nodus".to_owned()),
        ("FileVersion", version.clone()),
        ("InternalName", "Nodus".to_owned()),
        ("LegalCopyright", format!("Copyright © {} Saleem Hadad", build_year())),
        ("OriginalFilename", String::new()),
        ("ProductName", "Nodus".to_owned()),
        ("ProductVersion", format!("{version}.0")),
    ] {
        let _ = writeln!(rc, "      VALUE \"{key}\", \"{value}\"");
    }
    rc.push_str(
        "    END\n  END\n  BLOCK \"VarFileInfo\"\n  BEGIN\n    VALUE \"Translation\", 0x409, 1200\n  END\nEND\n",
    );

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("nodus-resources.rc");
    std::fs::write(&out, rc).expect("writing the resource script");
    if let Err(err) = embed_resource::compile(&out, embed_resource::NONE).manifest_optional() {
        panic!("Nodus.exe's resources didn't compile: {err}");
    }
}

/// The year now, or at `SOURCE_DATE_EPOCH` when a build sets it, in UTC.
fn build_year() -> i64 {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        });
    // Howard Hinnant's civil_from_days.
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}
