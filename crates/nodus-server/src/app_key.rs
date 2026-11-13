//! The GitHub App's private key, and the JWT (RS256) it signs to speak as
//! the App (SRV-062, SRV-063). GitHub gives the key as PKCS#1 PEM (an
//! `RSA PRIVATE KEY` block); PKCS#8 (a `PRIVATE KEY` block) is read too.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

/// What a key that can't be read is turned down with.
pub(crate) const NOT_A_KEY: &str = "That isn't a GitHub App private key: it's the .pem file GitHub downloads, starting -----BEGIN RSA PRIVATE KEY-----";

/// How far a JWT's clock is set back, for a GitHub whose clock is behind,
/// and how long it lasts: GitHub takes at most 10 minutes.
const BACKDATE_SECS: u64 = 60;
const LIFETIME_SECS: u64 = 9 * 60;

pub(crate) struct AppKey {
    #[cfg(not(windows))]
    pair: ring::signature::RsaKeyPair,
}

impl AppKey {
    /// Reads a PEM key.
    pub(crate) fn from_pem(pem: &str) -> Result<Self, String> {
        let (label, der) = decode_pem(pem).ok_or(NOT_A_KEY)?;
        Self::from_der(label, &der)
    }

    #[cfg(not(windows))]
    fn from_der(label: &str, der: &[u8]) -> Result<Self, String> {
        use ring::signature::RsaKeyPair;
        let pair = match label {
            "RSA PRIVATE KEY" => RsaKeyPair::from_der(der),
            "PRIVATE KEY" => RsaKeyPair::from_pkcs8(der),
            _ => return Err(NOT_A_KEY.to_owned()),
        };
        pair.map(|pair| Self { pair })
            .map_err(|err| format!("{NOT_A_KEY} ({err})"))
    }

    #[cfg(windows)]
    fn from_der(_: &str, _: &[u8]) -> Result<Self, String> {
        Err("A server's GitHub App key works on Linux and macOS".to_owned())
    }

    /// A JWT that speaks as App `app_id` for the next 9 minutes, `now`
    /// being seconds since the epoch.
    pub(crate) fn jwt(&self, app_id: u64, now: u64) -> Result<String, String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = format!(
            r#"{{"iat":{},"exp":{},"iss":"{app_id}"}}"#,
            now.saturating_sub(BACKDATE_SECS),
            now + LIFETIME_SECS
        );
        let signed = format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims));
        let signature = self.sign(signed.as_bytes())?;
        Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }

    #[cfg(not(windows))]
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
        let mut signature = vec![0u8; self.pair.public().modulus_len()];
        self.pair
            .sign(
                &ring::signature::RSA_PKCS1_SHA256,
                &ring::rand::SystemRandom::new(),
                message,
                &mut signature,
            )
            .map_err(|_| "Can't sign with the GitHub App's key".to_owned())?;
        Ok(signature)
    }

    #[cfg(windows)]
    fn sign(&self, _: &[u8]) -> Result<Vec<u8>, String> {
        Err("A server's GitHub App key works on Linux and macOS".to_owned())
    }
}

/// The label and bytes of the first PEM block: `RSA PRIVATE KEY` or
/// `PRIVATE KEY`.
fn decode_pem(pem: &str) -> Option<(&str, Vec<u8>)> {
    let begin = pem.find("-----BEGIN ")?;
    let rest = &pem[begin + "-----BEGIN ".len()..];
    let (label, rest) = rest.split_once("-----")?;
    let end = format!("-----END {label}-----");
    let (body, _) = rest.split_once(&end)?;
    let body: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let der = STANDARD.decode(body).ok()?;
    Some((label, der))
}

/// Seconds since the epoch.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A fresh 2048-bit key from `openssl genrsa`, never one kept in git:
    /// PKCS#1 from LibreSSL, PKCS#8 from OpenSSL 3.
    fn generated_key() -> String {
        let out = std::process::Command::new("openssl")
            .args(["genrsa", "2048"])
            .stderr(std::process::Stdio::null())
            .output()
            .expect("openssl is installed");
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn signs_a_jwt_github_can_check() {
        let pem = generated_key();
        let key = AppKey::from_pem(&pem).unwrap();
        let jwt = key.jwt(4242, 1_700_000_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({ "iat": 1_699_999_940, "exp": 1_700_000_540, "iss": "4242" })
        );
        // Checked with the key's public half, as GitHub checks it.
        let public = key.pair.public().as_ref().to_vec();
        let signed = format!("{}.{}", parts[0], parts[1]);
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, &public)
            .verify(signed.as_bytes(), &signature)
            .expect("a good signature");
    }

    #[test]
    fn turns_down_what_isnt_a_key() {
        for wrong in [
            "",
            "hello",
            "-----BEGIN RSA PRIVATE KEY-----\nnot base64!\n-----END RSA PRIVATE KEY-----", // Not keys. gitleaks:allow
            "-----BEGIN RSA PRIVATE KEY-----\naGVsbG8=\n-----END RSA PRIVATE KEY-----",    // Not keys. gitleaks:allow
            "-----BEGIN CERTIFICATE-----\naGVsbG8=\n-----END CERTIFICATE-----",
        ] {
            let err = AppKey::from_pem(wrong).err().expect("not a key");
            assert!(err.starts_with(NOT_A_KEY), "{wrong:?}: {err}");
        }
    }
}
