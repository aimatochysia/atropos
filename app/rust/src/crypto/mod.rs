//! Cryptographic building blocks for Atropos.
//!
//! Only FIPS-approved algorithms are used, apart from Argon2id for passwords
//! (RFC 9106, chosen because it resists GPU guessing far better than PBKDF2):
//!
//! | Role            | Algorithm                         | Standard            |
//! |-----------------|-----------------------------------|---------------------|
//! | Post-quantum KEM| ML-KEM-1024                       | FIPS 203            |
//! | Classical KEM   | ECDH P-384 (ephemeral-static)     | SP 800-56A r3       |
//! | KEM combiner    | HKDF-SHA-384                      | SP 800-56C r2       |
//! | AEAD            | AES-256-GCM                       | FIPS 197, SP 800-38D|
//! | Password KDF    | Argon2id                          | RFC 9106            |
//!
//! Every secret lives in a `Zeroizing` buffer and is wiped when dropped.
//! Algorithm choices are fixed here; nothing is negotiated from untrusted input.

// Parts of this API are only used by tests until the vault format lands (v0.1).
#![allow(dead_code)]

pub mod aead;
pub mod hybrid;
pub mod kdf;
pub mod password;
pub mod stream;

/// Padmé padding (Nikitin et al., PETS 2019): rounds a length up so that at
/// most O(log log L) bits of it leak, with at most ~12% overhead.
pub fn padme(len: u64) -> u64 {
    if len < 2 {
        return len;
    }
    let e = 63 - len.leading_zeros() as u64; // floor(log2 len)
    let s = (64 - e.leading_zeros() as u64).max(1); // floor(log2 e) + 1
    let last_bits = e - s;
    let mask = (1u64 << last_bits) - 1;
    (len + mask) & !mask
}

/// HMAC-SHA-384 over `data`.
pub fn hmac_sha384(key: &[u8], data: &[&[u8]]) -> Vec<u8> {
    use aws_lc_rs::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA384, key);
    let mut ctx = hmac::Context::with_key(&key);
    for d in data {
        ctx.update(d);
    }
    ctx.sign().as_ref().to_vec()
}

/// Constant-time HMAC-SHA-384 check.
pub fn hmac_sha384_verify(key: &[u8], data: &[&[u8]], tag: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    let expected = hmac_sha384(key, data);
    expected.len() == tag.len() && bool::from(expected.ct_eq(tag))
}

/// SHA-384 digest.
pub fn sha384(data: &[&[u8]]) -> Vec<u8> {
    let mut ctx = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA384);
    for d in data {
        ctx.update(d);
    }
    ctx.finish().as_ref().to_vec()
}

use aws_lc_rs::rand::{SecureRandom, SystemRandom};

use crate::error::{CoreError, CoreResult};

/// Fills `buf` from the operating system's CSPRNG.
pub fn random_bytes(buf: &mut [u8]) -> CoreResult<()> {
    SystemRandom::new().fill(buf).map_err(|_| CoreError::Crypto)
}

/// A fresh random array, e.g. a salt or a key.
pub fn random_array<const N: usize>() -> CoreResult<[u8; N]> {
    let mut out = [0u8; N];
    random_bytes(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padme_values() {
        assert_eq!(padme(0), 0);
        assert_eq!(padme(1), 1);
        assert_eq!(padme(9), 10);
        assert_eq!(padme(1000), 1024);
        assert_eq!(padme(1025), 1088);
        for len in [3u64, 100, 4097, 1_000_000, 123_456_789] {
            let p = padme(len);
            assert!(
                p >= len && (p - len) as f64 <= len as f64 * 0.12 + 1.0,
                "{len} -> {p}"
            );
        }
    }

    #[test]
    fn hmac_round_trip() {
        let tag = hmac_sha384(b"k", &[b"a", b"b"]);
        assert!(hmac_sha384_verify(b"k", &[b"ab"], &tag));
        assert!(!hmac_sha384_verify(b"k", &[b"ac"], &tag));
    }
}
