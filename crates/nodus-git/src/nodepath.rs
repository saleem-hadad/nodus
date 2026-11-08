//! The parts of Node's `path` module git.ts and voice.ts lean on, with Node's
//! rules rather than Rust's: `path.resolve`, `path.relative` and
//! `path.isAbsolute` for the diff's containment check (GIT-015), and
//! `path.parse(p).name` for voice's hint words (VOICE-012).
//!
//! Node picks the POSIX or the Windows flavour by platform (PD-6, PD-8); both
//! are here, chosen with [`Flavor::NATIVE`], so either can be tested anywhere.
//! Paths are lexical: nothing touches the file system.

/// Which of Node's `path` modules: `path.posix` or `path.win32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Posix,
    Win32,
}

impl Flavor {
    /// The one Node uses on this platform.
    pub const NATIVE: Self = if cfg!(windows) { Self::Win32 } else { Self::Posix };

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == Self::Win32 && c == '\\')
    }

    fn sep(self) -> &'static str {
        match self {
            Self::Posix => "/",
            Self::Win32 => "\\",
        }
    }
}

/// The root a path starts from, and whether it's absolute on its own.
struct Root<'a> {
    /// `C:` or `\\server\share` on Windows; empty otherwise.
    device: String,
    /// Starts at a root (`/`, `C:\`, `\\server\share`), not relative.
    absolute: bool,
    rest: &'a str,
}

fn root(flavor: Flavor, p: &str) -> Root<'_> {
    match flavor {
        Flavor::Posix => match p.strip_prefix('/') {
            Some(rest) => Root {
                device: String::new(),
                absolute: true,
                rest,
            },
            None => Root {
                device: String::new(),
                absolute: false,
                rest: p,
            },
        },
        Flavor::Win32 => win32_root(p),
    }
}

fn win32_root(p: &str) -> Root<'_> {
    let is_sep = |c: char| c == '/' || c == '\\';
    let chars: Vec<char> = p.chars().take(3).collect();
    // A drive: `C:` or `C:\`.
    if chars.len() >= 2 && chars[0].is_ascii_alphabetic() && chars[1] == ':' {
        let absolute = chars.get(2).is_some_and(|&c| is_sep(c));
        return Root {
            device: p[..2].to_owned(),
            absolute,
            rest: &p[2..],
        };
    }
    if p.starts_with(is_sep) {
        // A UNC root: two separators, a server and a share.
        let body = p.trim_start_matches(is_sep);
        let leading = p.len() - body.len();
        if leading == 2 {
            let mut parts = body.splitn(3, is_sep);
            if let (Some(server), Some(share)) = (parts.next(), parts.next())
                && !server.is_empty()
                && !share.is_empty()
            {
                let consumed = 2 + server.len() + 1 + share.len();
                return Root {
                    device: format!("\\\\{server}\\{share}"),
                    absolute: true,
                    rest: &p[consumed..],
                };
            }
        }
        return Root {
            device: String::new(),
            absolute: true,
            rest: body,
        };
    }
    Root {
        device: String::new(),
        absolute: false,
        rest: p,
    }
}

/// Folds `.` and `..` away. `..` never climbs above the root.
fn normalize<'a>(flavor: Flavor, parts: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for part in parts {
        for segment in part.split(|c| flavor.is_sep(c)) {
            match segment {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                _ => out.push(segment),
            }
        }
    }
    out
}

/// A resolved absolute path, split into its device and segments.
struct Resolved<'a> {
    device: String,
    segments: Vec<&'a str>,
}

impl Resolved<'_> {
    fn render(&self, flavor: Flavor) -> String {
        format!("{}{}{}", self.device, flavor.sep(), self.segments.join(flavor.sep()))
    }
}

/// `path.resolve(base, p)` with an absolute `base`.
fn resolve_parts<'a>(flavor: Flavor, base: &'a str, p: &'a str) -> Resolved<'a> {
    let target = root(flavor, p);
    let from = root(flavor, base);
    let same_device = target.device.eq_ignore_ascii_case(&from.device);
    let (device, parts): (String, Vec<&str>) =
        if target.absolute && (!target.device.is_empty() || flavor == Flavor::Posix) {
            (target.device.clone(), vec![target.rest])
        } else if target.absolute {
            // Rooted without a drive: on the base's drive.
            (from.device.clone(), vec![target.rest])
        } else if target.device.is_empty() || same_device {
            (from.device.clone(), vec![from.rest, target.rest])
        } else {
            // `D:foo` from a base on another drive: that drive's root.
            (target.device.clone(), vec![target.rest])
        };
    Resolved {
        device,
        segments: normalize(flavor, parts),
    }
}

/// `path.resolve(base, p)`, for an absolute `base`.
pub fn resolve(flavor: Flavor, base: &str, p: &str) -> String {
    resolve_parts(flavor, base, p).render(flavor)
}

/// `path.isAbsolute(p)`.
pub fn is_absolute(flavor: Flavor, p: &str) -> bool {
    match flavor {
        Flavor::Posix => p.starts_with('/'),
        Flavor::Win32 => {
            let mut chars = p.chars();
            match (chars.next(), chars.next(), chars.next()) {
                (Some(c), _, _) if flavor.is_sep(c) => true,
                (Some(d), Some(':'), Some(c)) => d.is_ascii_alphabetic() && flavor.is_sep(c),
                _ => false,
            }
        }
    }
}

/// `path.relative(from, path.resolve(from, p))`, for an absolute `from`:
/// the way from `from` to `p`. Windows compares without case, and a path on
/// another drive comes back absolute.
pub fn relative_to(flavor: Flavor, base: &str, p: &str) -> String {
    let from = resolve_parts(flavor, base, ".");
    let to = resolve_parts(flavor, base, p);
    let eq = |a: &str, b: &str| match flavor {
        Flavor::Posix => a == b,
        Flavor::Win32 => a.to_lowercase() == b.to_lowercase(),
    };
    if !eq(&from.device, &to.device) {
        return to.render(flavor);
    }
    let common = from
        .segments
        .iter()
        .zip(&to.segments)
        .take_while(|(a, b)| eq(a, b))
        .count();
    let mut out: Vec<&str> = vec![".."; from.segments.len() - common];
    out.extend(&to.segments[common..]);
    out.join(flavor.sep())
}

/// `path.parse(p).name`: the last segment without its extension, by Node's
/// rules (`.gitignore` has none, `a.b.c` is `a.b`, `..` is itself).
pub fn parse_name(flavor: Flavor, p: &str) -> String {
    // A drive is the root on Windows, not part of the name.
    let p = match flavor {
        Flavor::Win32 => {
            let r = win32_root(p);
            if r.device.len() == 2 { r.rest } else { p }
        }
        Flavor::Posix => p,
    };
    let units: Vec<u16> = p.encode_utf16().collect();
    let is_sep = |u: u16| u == u16::from(b'/') || (flavor == Flavor::Win32 && u == u16::from(b'\\'));
    let dot = u16::from(b'.');
    let mut start_dot: Option<usize> = None;
    let mut start_part = 0;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    // 0: nothing before the dot yet; 1: a dot; -1: something else.
    let mut pre_dot_state = 0;
    for i in (0..units.len()).rev() {
        let code = units[i];
        if is_sep(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
        if code == dot {
            if start_dot.is_none() {
                start_dot = Some(i);
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot.is_some() {
            pre_dot_state = -1;
        }
    }
    let Some(end) = end else {
        return String::new();
    };
    let whole = |from: usize, to: usize| String::from_utf16_lossy(&units[from..to]);
    match start_dot {
        Some(dot_at)
            if pre_dot_state != 0 && !(pre_dot_state == 1 && dot_at == end - 1 && dot_at == start_part + 1) =>
        {
            whole(start_part, dot_at)
        }
        _ => whole(start_part, end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Flavor::{Posix, Win32};

    #[test]
    fn resolves_as_node_does() {
        assert_eq!(resolve(Posix, "/repo", "a/b.txt"), "/repo/a/b.txt");
        assert_eq!(resolve(Posix, "/repo", "../outside.txt"), "/outside.txt");
        assert_eq!(resolve(Posix, "/repo", "/etc/hosts"), "/etc/hosts");
        assert_eq!(resolve(Posix, "/repo", "."), "/repo");
        assert_eq!(resolve(Posix, "/", "../../x"), "/x");
        assert_eq!(
            resolve(Win32, "C:/Users/example/repo", "a/b.txt"),
            "C:\\Users\\example\\repo\\a\\b.txt"
        );
        assert_eq!(resolve(Win32, "C:\\repo", "\\etc\\hosts"), "C:\\etc\\hosts");
        assert_eq!(resolve(Win32, "C:\\repo", "D:\\x"), "D:\\x");
        assert_eq!(
            resolve(Win32, "\\\\server\\share\\repo", "a"),
            "\\\\server\\share\\repo\\a"
        );
    }

    #[test]
    fn finds_the_way_between_paths() {
        assert_eq!(relative_to(Posix, "/repo", "a.txt"), "a.txt");
        assert_eq!(relative_to(Posix, "/repo", "."), "");
        assert_eq!(relative_to(Posix, "/repo", "../outside.txt"), "../outside.txt");
        assert_eq!(relative_to(Posix, "/repo", "/etc/hosts"), "../etc/hosts");
        assert_eq!(relative_to(Posix, "/repo", "..foo/bar"), "..foo/bar");
        assert_eq!(relative_to(Posix, "/repo", "a/../../repo/b"), "b");
        assert_eq!(relative_to(Win32, "C:/repo", "A.txt"), "A.txt");
        assert_eq!(relative_to(Win32, "C:/Repo", "c:\\repo\\x"), "x");
        assert_eq!(relative_to(Win32, "C:/repo", "D:\\x"), "D:\\x");
        assert_eq!(relative_to(Win32, "C:/repo", "..\\x"), "..\\x");
    }

    #[test]
    fn tells_absolute_paths() {
        assert!(is_absolute(Posix, "/a"));
        assert!(!is_absolute(Posix, "a"));
        assert!(!is_absolute(Posix, "C:\\a"));
        assert!(is_absolute(Win32, "C:\\a"));
        assert!(is_absolute(Win32, "C:/a"));
        assert!(is_absolute(Win32, "\\a"));
        assert!(!is_absolute(Win32, "C:a"));
        assert!(!is_absolute(Win32, "a"));
    }

    #[test]
    fn names_files_as_path_parse_does() {
        let cases = [
            ("src/BoardView.tsx", "BoardView"),
            ("a.b.c", "a.b"),
            (".gitignore", ".gitignore"),
            ("dir/.env.local", ".env"),
            ("..", ".."),
            ("...", ".."),
            ("..a", "."),
            ("a.", "a"),
            ("Makefile", "Makefile"),
            ("dir/", "dir"),
            ("", ""),
            ("/", ""),
        ];
        for (path, name) in cases {
            assert_eq!(parse_name(Posix, path), name, "{path}");
        }
        assert_eq!(parse_name(Posix, "a\\b.txt"), "a\\b");
        assert_eq!(parse_name(Win32, "a\\b.txt"), "b");
        assert_eq!(parse_name(Win32, "C:x.rs"), "x");
    }
}
