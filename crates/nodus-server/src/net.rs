//! Who may reach a server: where it listens (SRV-020), the names it answers
//! to (SRV-021), and the token a connection carries (SRV-022).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use hyper::header::{AUTHORIZATION, HeaderMap};
use nodus_wire::PORT;
use serde_json::Value;

/// How long `tailscale` gets to answer.
const TAILSCALE_WAIT: Duration = Duration::from_secs(3);

/// This machine on the tailnet, as `tailscale` says at startup: nothing
/// when it isn't installed or isn't up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tailscale {
    /// `tailscale ip -4`.
    pub ip: Option<Ipv4Addr>,
    /// Its MagicDNS name, `nodus.tail1234.ts.net`, and the short one, `nodus`.
    pub names: Vec<String>,
}

impl Tailscale {
    pub(crate) async fn detect() -> Self {
        let (ip, status) = tokio::join!(tailscale(&["ip", "-4"]), tailscale(&["status", "--json"]));
        Self {
            ip: ip.as_deref().and_then(parse_ip),
            names: status.as_deref().map(parse_names).unwrap_or_default(),
        }
    }
}

/// What `tailscale <args>` prints, when it runs and succeeds in time.
pub(crate) async fn tailscale(args: &[&str]) -> Option<String> {
    let run = tokio::process::Command::new("tailscale")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(TAILSCALE_WAIT, run).await.ok()?.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The first line of `tailscale ip -4`.
fn parse_ip(stdout: &str) -> Option<Ipv4Addr> {
    stdout.lines().next()?.trim().parse().ok()
}

/// `Self.DNSName` from `tailscale status --json`, without its final dot,
/// and its first label.
fn parse_names(json: &str) -> Vec<String> {
    let Ok(status) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let dns = status["Self"]["DNSName"].as_str().unwrap_or("");
    let dns = dns.trim_end_matches('.').to_ascii_lowercase();
    if dns.is_empty() {
        return Vec::new();
    }
    let short = dns.split('.').next().unwrap_or("").to_owned();
    let mut names = vec![dns];
    if short != names[0] {
        names.push(short);
    }
    names
}

/// SRV-020: where `serve` listens. Port 7420 of the machine's Tailscale
/// address when Tailscale is up, else of `127.0.0.1`, for an SSH tunnel.
/// `--bind <ip>` or `--bind <ip>:<port>` picks another, but only loopback
/// or the Tailscale address: there's no TLS (SRV-021), so never `0.0.0.0`.
pub(crate) fn bind_address(requested: Option<&str>, tailscale: Option<Ipv4Addr>) -> Result<SocketAddr, String> {
    let Some(requested) = requested else {
        let ip = tailscale.unwrap_or(Ipv4Addr::LOCALHOST);
        return Ok(SocketAddr::new(IpAddr::V4(ip), PORT));
    };
    let address = requested
        .parse::<SocketAddr>()
        .or_else(|_| requested.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, PORT)))
        .map_err(|_| format!("--bind {requested}: not an IP address"))?;
    let ip = address.ip();
    if ip.is_loopback() || tailscale.is_some_and(|tailscale| ip == IpAddr::V4(tailscale)) {
        return Ok(address);
    }
    let allowed = match tailscale {
        Some(tailscale) => format!("loopback or {tailscale}, its Tailscale address"),
        None => "loopback".to_owned(),
    };
    Err(format!(
        "--bind {ip}: a server listens only on {allowed}, since it has no TLS"
    ))
}

/// SRV-021: the names a request may call this server by, so a web page
/// whose own name resolves to this machine (DNS rebinding) is turned away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hosts(Vec<String>);

impl Hosts {
    /// `localhost`, `127.0.0.1`, the Tailscale address and names, and the
    /// address the server listens on.
    pub(crate) fn new(tailscale: &Tailscale, listening: IpAddr) -> Self {
        let mut names = vec!["localhost".to_owned(), Ipv4Addr::LOCALHOST.to_string()];
        names.extend(tailscale.ip.map(|ip| ip.to_string()));
        names.extend(tailscale.names.iter().cloned());
        names.push(listening.to_string());
        names.sort_unstable();
        names.dedup();
        Self(names)
    }

    /// Whether a request's `Host` names this server, with or without a port.
    pub(crate) fn allows(&self, host: Option<&str>) -> bool {
        let Some(host) = host else {
            return false;
        };
        let name = match host.strip_prefix('[') {
            // `[::1]:7420`
            Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
            None => host.rsplit_once(':').map_or(host, |(name, _)| name),
        };
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        self.0.contains(&name)
    }
}

/// The token in `Authorization: Bearer <token>`: the only place a server
/// reads one, never a query string or a cookie (SRV-022).
pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// Whether two tokens are the same, taking as long whichever byte differs.
pub(crate) fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |differ, (a, b)| differ | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAILNET: Ipv4Addr = Ipv4Addr::new(100, 101, 102, 103);

    #[test]
    fn listens_on_the_tailscale_address_else_loopback() {
        assert_eq!(
            bind_address(None, Some(TAILNET)).unwrap(),
            "100.101.102.103:7420".parse().unwrap()
        );
        assert_eq!(bind_address(None, None).unwrap(), "127.0.0.1:7420".parse().unwrap());
    }

    #[test]
    fn binds_only_loopback_or_the_tailscale_address() {
        assert_eq!(
            bind_address(Some("127.0.0.1:0"), None).unwrap(),
            "127.0.0.1:0".parse().unwrap()
        );
        assert_eq!(bind_address(Some("::1"), None).unwrap(), "[::1]:7420".parse().unwrap());
        assert_eq!(
            bind_address(Some("100.101.102.103"), Some(TAILNET)).unwrap(),
            "100.101.102.103:7420".parse().unwrap()
        );
        for refused in ["0.0.0.0", "0.0.0.0:7420", "::", "192.168.1.5", "100.101.102.104"] {
            let err = bind_address(Some(refused), Some(TAILNET)).unwrap_err();
            assert!(err.contains("listens only on loopback"), "{refused}: {err}");
        }
        // Without Tailscale, its old address is just another address.
        assert!(bind_address(Some("100.101.102.103"), None).is_err());
        assert!(
            bind_address(Some("nodus"), None)
                .unwrap_err()
                .contains("not an IP address")
        );
    }

    #[test]
    fn reads_what_tailscale_says() {
        assert_eq!(parse_ip("100.101.102.103\n"), Some(TAILNET));
        assert_eq!(parse_ip(""), None);
        let status = r#"{"Self":{"HostName":"Nodus-1","DNSName":"Nodus-1.tail1234.ts.net."},"Peer":{}}"#;
        assert_eq!(parse_names(status), ["nodus-1.tail1234.ts.net", "nodus-1"]);
        assert!(parse_names(r#"{"Self":{"DNSName":""}}"#).is_empty());
        assert!(parse_names("not json").is_empty());
    }

    #[test]
    fn answers_only_to_its_own_names() {
        let tailscale = Tailscale {
            ip: Some(TAILNET),
            names: vec!["nodus.tail1234.ts.net".into(), "nodus".into()],
        };
        let hosts = Hosts::new(&tailscale, IpAddr::V4(TAILNET));
        for allowed in [
            "localhost",
            "localhost:7420",
            "127.0.0.1:7420",
            "100.101.102.103:7420",
            "nodus:7420",
            "NODUS.tail1234.ts.net.",
        ] {
            assert!(hosts.allows(Some(allowed)), "{allowed}");
        }
        for refused in ["evil.example:7420", "nodus.evil.example", "10.0.0.1", "", "[::1]:7420"] {
            assert!(!hosts.allows(Some(refused)), "{refused}");
        }
        assert!(!hosts.allows(None));
        // Listening on ::1, it answers to it.
        assert!(Hosts::new(&Tailscale::default(), "::1".parse().unwrap()).allows(Some("[::1]:7420")));
    }

    #[test]
    fn reads_the_token_from_the_authorization_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(AUTHORIZATION, "bearer abc".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abd", "abc"));
        assert!(!same_token("ab", "abc"));
    }
}
