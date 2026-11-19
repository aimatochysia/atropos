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
