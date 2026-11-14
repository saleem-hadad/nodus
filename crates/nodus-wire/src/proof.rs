//! A server proving it's the one a secret was made for, before the secret
//! goes to it (SRV-030). The wire is plain HTTP (SRV-021), and anything can
//! answer at a server's address: another device on a network where its name
//! resolves somewhere else, or, while `serve` restarts, a session on the
//! server itself that took its port. Neither may be handed a device's
//! token or a setup token. (The setup code goes without: a server that
//! proved it keeps it would answer guesses past its 10 tries an hour.)
//!
//! The server keeps only each secret's SHA-256 ([`secret_hash`]): a device
//! token's, a setup token's. So before a request that carries one, the
//! client asks, on the same connection,
//!
//! ```text
//! GET /api/prove?nonce=<64 hex>&hint=<16 hex>
//! ```
//!
//! and the server answers `{ "proof": HMAC-SHA256(hash, "nodus-server-proof\n" + nonce) }`
//! for the secret whose [`hint`] that is. Only a server that holds the hash
//! can, and the client checks it before it sends the secret. The hint is a
//! hash of the hash, so it gives nothing away, and a fresh nonce each time
//! means no answer can be replayed.

use sha2::{Digest, Sha256};

/// `GET`, with no token: a [`Proof`] for one secret (SRV-030).
pub const PROVE_PATH: &str = "/api/prove";

/// What `GET /api/prove` answers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Proof {
    pub proof: String,
}

/// What a server keeps of a secret: its SHA-256, as lower-case hex.
pub fn secret_hash(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// Which secret a proof is asked for, by its kept hash: the first 16 hex of
/// that hash's own SHA-256.
pub fn hint(hash: &str) -> String {
    let mut hint = secret_hash(hash);
    hint.truncate(16);
    hint
}

/// The proof for `nonce` from a server that keeps `hash`.
pub fn proof(hash: &str, nonce: &str) -> String {
    hex(&hmac_sha256(
        hash.as_bytes(),
        format!("nodus-server-proof\n{nonce}").as_bytes(),
    ))
}

/// Whether `given` is the proof for `nonce` of a server that keeps `hash`,
/// taking as long whichever byte differs.
pub fn proves(hash: &str, nonce: &str, given: &str) -> bool {
    let (given, expected) = (given.as_bytes(), proof(hash, nonce));
    let expected = expected.as_bytes();
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |differ, (a, b)| differ | (a ^ b)) == 0
}

/// A nonce or hint as the server takes them: lower-case hex of a sensible
/// length.
pub fn is_hex(text: &str, lengths: std::ops::RangeInclusive<usize>) -> bool {
    lengths.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.iter().map(|b| b ^ byte).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proves_only_with_the_hash_and_the_nonce() {
        let hash = secret_hash("a-device-token");
        assert_eq!(hash.len(), 64);
        let nonce = "00".repeat(32);
        let given = proof(&hash, &nonce);
        assert!(proves(&hash, &nonce, &given));
        assert!(!proves(&hash, &"11".repeat(32), &given));
        assert!(!proves(&secret_hash("another"), &nonce, &given));
        assert!(!proves(&hash, &nonce, &given[..63]));
        assert_eq!(hint(&hash).len(), 16);
        assert_ne!(hint(&hash), hash[..16]);
    }

    #[test]
    fn matches_rfc_4231s_hmac() {
        // RFC 4231, test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn takes_only_lower_case_hex() {
        assert!(is_hex(&"ab".repeat(32), 32..=128));
        for wrong in ["AB", "zz", ""] {
            assert!(!is_hex(wrong, 1..=128), "{wrong}");
        }
    }
}
