//! Reading GitHub's answer, and comparing versions, as `updater.ts` does.

use serde::Deserialize;

use crate::env::Platform;

/// The newest release that isn't a draft or a prerelease.
pub const LATEST_URL: &str = "https://api.github.com/repos/saleem-hadad/nodus/releases/latest";
/// Where Settings sends someone to download it by hand (UPD-085).
pub const RELEASES_URL: &str = "https://github.com/saleem-hadad/nodus/releases/latest";

/// A release with a finished build for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// `x.y.z`, from the tag without its `v`.
    pub version: String,
    pub url: String,
    pub size: u64,
    /// The lowercase hex SHA-256 GitHub computed for the file, when it did.
    pub sha256: Option<String>,
}

/// The fields of `releases/latest` that matter; anything else is ignored,
/// and a field of another type counts as missing.
#[derive(Debug, Deserialize)]
struct Body {
    #[serde(default, deserialize_with = "lenient")]
    tag_name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    assets: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: f64,
    #[serde(default, deserialize_with = "lenient")]
    digest: Option<String>,
    browser_download_url: String,
}

/// A value of the wrong type reads as missing.
fn lenient<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(de)?;
    Ok(serde_json::from_value(value).ok())
}

/// UPD-021: the file this machine takes. An arm64 machine gets the arm64
/// build even when this copy is the Intel one. Names as electron-builder.yml.
pub fn asset_name(platform: Platform, arm64: bool) -> String {
    let arch = if arm64 { "arm64" } else { "x64" };
    match platform {
        Platform::Windows => format!("Nodus-Setup-{arch}.exe"),
        _ => format!("Nodus-{arch}.dmg"),
    }
}

/// UPD-020…UPD-024: the release in GitHub's answer, or None while it has
/// nothing finished for this machine (it may still be uploading) or its tag
/// isn't a plain version. Err for a body that isn't JSON.
pub fn parse_release(json: &str, asset: &str) -> Result<Option<Release>, serde_json::Error> {
    let body: Body = serde_json::from_str(json)?;
    let found = body
        .assets
        .unwrap_or_default()
        .into_iter()
        .filter_map(|asset| serde_json::from_value::<Asset>(asset).ok())
        .find(|candidate| candidate.name == asset && candidate.state == "uploaded");
    // The version names a folder that gets deleted, so it has to be a plain one.
    let version = body.tag_name.as_deref().and_then(tag_version);
    let (Some(version), Some(asset)) = (version, found) else {
        return Ok(None);
    };
    Ok(Some(Release {
        version: version.to_owned(),
        url: asset.browser_download_url,
        size: asset.size as u64,
        sha256: asset.digest.as_deref().and_then(digest_sha256).map(str::to_owned),
    }))
}

/// `/^v?(\d+\.\d+\.\d+)$/` (UPD-022).
pub fn tag_version(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    plain_version(version).then_some(version)
}

/// `\d+\.\d+\.\d+`, ASCII digits only.
fn plain_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// `/^sha256:([0-9a-f]{64})$/`, lowercase hex only (UPD-024).
pub fn digest_sha256(digest: &str) -> Option<&str> {
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(hex)
}

/// `/^Nodus-Setup-(\d+\.\d+\.\d+)\.exe$/` (UPD-072).
pub fn installer_version(name: &str) -> Option<&str> {
    let version = name.strip_prefix("Nodus-Setup-")?.strip_suffix(".exe")?;
    plain_version(version).then_some(version)
}

/// UPD-025: whether version `a` comes after `b`, by major.minor.patch, each
/// part read as `parseInt(n, 10) || 0`.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<f64> { v.split('.').map(parse_int).collect() };
    let (x, y) = (parts(a), parts(b));
    for i in 0..3 {
        let (p, q) = (x.get(i).copied().unwrap_or(0.0), y.get(i).copied().unwrap_or(0.0));
        if p != q {
            return p > q;
        }
    }
    false
}

/// `parseInt(text, 10) || 0`: leading whitespace, a sign, then the digits
/// up to the first other character; 0 when there are none.
fn parse_int(text: &str) -> f64 {
    let text = js_trim_start(text);
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => (-1.0, &text[1..]),
        Some(b'+') => (1.0, &text[1..]),
        _ => (1.0, text),
    };
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return 0.0;
    }
    let value = digits.parse::<f64>().unwrap_or(0.0) * sign;
    // `-0 || 0` is 0.
    if value == 0.0 { 0.0 } else { value }
}

/// JavaScript's whitespace and line terminators, which include U+FEFF.
pub(crate) fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_space)
}

/// `text.trim()` as JavaScript trims.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// UPD-072: the newest installer newer than `current` among `names`.
pub fn newest_installer<'a>(names: impl IntoIterator<Item = &'a str>, current: &str) -> Option<&'a str> {
    let mut newest: Option<(&str, &str)> = None;
    for name in names {
        if let Some(version) = installer_version(name)
            && is_newer(version, newest.map_or(current, |(_, newest)| newest))
        {
            newest = Some((name, version));
        }
    }
    newest.map(|(name, _)| name)
}

/// UPD-046: the team in `codesign -dv`'s stderr (`/^TeamIdentifier=(.+)$/m`),
/// or None for `not set` (an ad-hoc signature) or no line at all.
pub fn team_identifier(stderr: &str) -> Option<&str> {
    let team = stderr
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .find_map(|line| line.strip_prefix("TeamIdentifier=").filter(|team| !team.is_empty()))?;
    (team != "not set").then_some(team)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "9112ae3600000000000000000000000000000000000000000000000000000abc";

    fn answer(tag: &str, assets: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","draft":false,"assets":{assets}}}"#)
    }

    fn asset(name: &str, state: &str, digest: &str) -> String {
        format!(
            r#"{{"name":"{name}","state":"{state}","size":136446713,"digest":{digest},"browser_download_url":"https://github.com/saleem-hadad/nodus/releases/download/v0.9.5/{name}","label":null}}"#
        )
    }

    #[test]
    fn names_the_build_for_this_machine() {
        assert_eq!(asset_name(Platform::Mac, true), "Nodus-arm64.dmg");
        assert_eq!(asset_name(Platform::Mac, false), "Nodus-x64.dmg");
        assert_eq!(asset_name(Platform::Windows, false), "Nodus-Setup-x64.exe");
        assert_eq!(asset_name(Platform::Windows, true), "Nodus-Setup-arm64.exe");
    }

    #[test]
    fn reads_the_release_for_this_machine() {
        let json = answer(
            "v0.9.5",
            &format!(
                "[{},{},{}]",
                asset("Nodus-x64.dmg", "uploaded", &format!("\"sha256:{SHA}\"")),
                asset("Nodus-arm64.dmg", "open", "null"),
                asset("Nodus-arm64.dmg", "uploaded", &format!("\"sha256:{SHA}\""))
            ),
        );
        let release = parse_release(&json, "Nodus-arm64.dmg").unwrap().unwrap();
        assert_eq!(
            release,
            Release {
                version: "0.9.5".into(),
                url: "https://github.com/saleem-hadad/nodus/releases/download/v0.9.5/Nodus-arm64.dmg".into(),
                size: 136_446_713,
                sha256: Some(SHA.into()),
            }
        );
    }

    #[test]
    fn has_nothing_for_this_machine_without_a_finished_build_or_a_plain_tag() {
        let uploading = answer("v0.9.5", &format!("[{}]", asset("Nodus-arm64.dmg", "open", "null")));
        assert_eq!(parse_release(&uploading, "Nodus-arm64.dmg").unwrap(), None);
        let other = answer("v0.9.5", &format!("[{}]", asset("Nodus-x64.dmg", "uploaded", "null")));
        assert_eq!(parse_release(&other, "Nodus-arm64.dmg").unwrap(), None);
        let asset = format!("[{}]", asset("Nodus-arm64.dmg", "uploaded", "null"));
        for tag in ["v1.0.0-beta.1", "1.0", "v1.0.0.1", "release-1", "", "V1.0.0"] {
            assert_eq!(
                parse_release(&answer(tag, &asset), "Nodus-arm64.dmg").unwrap(),
                None,
                "{tag}"
            );
        }
        assert_eq!(parse_release("{}", "Nodus-arm64.dmg").unwrap(), None);
        assert_eq!(
            parse_release(r#"{"tag_name":5,"assets":"x"}"#, "Nodus-arm64.dmg").unwrap(),
            None
        );
        assert!(parse_release("<!DOCTYPE html>", "Nodus-arm64.dmg").is_err());
    }

    #[test]
    fn takes_a_tag_with_or_without_its_v() {
        assert_eq!(tag_version("v0.9.4"), Some("0.9.4"));
        assert_eq!(tag_version("0.10.0"), Some("0.10.0"));
        assert_eq!(tag_version("v٣.0.0"), None);
    }

    #[test]
    fn reads_only_lowercase_sha256_digests() {
        assert_eq!(digest_sha256(&format!("sha256:{SHA}")), Some(SHA));
        assert_eq!(digest_sha256(&format!("sha256:{}", SHA.to_uppercase())), None);
        assert_eq!(digest_sha256("sha256:abc"), None);
        assert_eq!(digest_sha256(&format!("sha512:{SHA}")), None);
        let json = answer(
            "v0.9.5",
            &format!("[{}]", asset("Nodus-x64.dmg", "uploaded", "\"md5:1\"")),
        );
        assert_eq!(parse_release(&json, "Nodus-x64.dmg").unwrap().unwrap().sha256, None);
    }

    #[test]
    fn compares_versions_as_parse_int_reads_them() {
        assert!(is_newer("0.9.5", "0.9.4"));
        assert!(is_newer("0.10.0", "0.9.4"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.9.4", "0.9.4"));
        assert!(!is_newer("0.9.3", "0.9.4"));
        assert!(!is_newer("0.9.4.1", "0.9.4"), "only three parts count");
        assert!(is_newer("1", "0.9.9"));
        assert!(!is_newer("0.9", "0.9.0"));
        assert!(is_newer("0.9.5abc", "0.9.4"));
        assert!(!is_newer("x.y.z", "0.0.0"));
        assert!(is_newer(" 1.0.0", "0.9.0"));
        assert!(!is_newer("-1.0.0", "0.0.0"));
    }

    #[test]
    fn keeps_the_newest_newer_installer() {
        let names = [
            "Nodus-Setup-0.8.0.exe",
            "Nodus-Setup-0.8.9.exe",
            "Nodus-Setup-0.9.1.exe",
            "Nodus-Setup-0.9.0.exe",
            "stray.tmp",
            "Nodus-Setup-1.0.exe",
        ];
        assert_eq!(newest_installer(names, "0.8.9"), Some("Nodus-Setup-0.9.1.exe"));
        assert_eq!(newest_installer(names, "0.9.1"), None);
        assert_eq!(installer_version("Nodus-Setup-0.9.0.exe"), Some("0.9.0"));
        assert_eq!(installer_version("Nodus-Setup-x64.exe"), None);
    }

    #[test]
    fn reads_the_team_from_codesign() {
        let signed = "Executable=/Applications/Nodus.app/Contents/MacOS/Nodus\nIdentifier=dev.saleem.nodus\n\
                      Format=app bundle with Mach-O thin (arm64)\nTeamIdentifier=ABCDE12345\nSealed Resources version=2\n";
        assert_eq!(team_identifier(signed), Some("ABCDE12345"));
        assert_eq!(team_identifier("Identifier=x\nTeamIdentifier=not set\n"), None);
        assert_eq!(team_identifier("Identifier=x\r\nTeamIdentifier=ABC\r\n"), Some("ABC"));
        assert_eq!(team_identifier("TeamIdentifier=\n"), None);
        assert_eq!(team_identifier("no team here"), None);
    }

    #[test]
    fn trims_as_javascript_does() {
        assert_eq!(js_trim("\u{feff} 0.9.5\n"), "0.9.5");
    }
}
