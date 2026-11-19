//! Password-based key derivation with Argon2id (RFC 9106).
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

pub const SALT_LEN: usize = 16;

/// Argon2id cost. Stored next to each password slot so a vault made on a
/// fast desktop still opens on a phone (just more slowly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Argon2Cost {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Argon2Cost {
    /// The floor Atropos never goes below: 64 MiB, 3 passes (RFC 9106 second
    /// recommended option).
    pub const MINIMUM: Argon2Cost = Argon2Cost {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 1,
    };

    /// The default target from the plan: 256 MiB.
    pub const DEFAULT: Argon2Cost = Argon2Cost {
        memory_kib: 256 * 1024,
        iterations: 3,
        parallelism: 1,
    };

    /// Refuses costs below the floor or absurdly high ones (a crafted vault
    /// header must not be able to exhaust memory).
    pub fn validate(&self) -> CoreResult<()> {
        let ok = self.memory_kib >= Self::MINIMUM.memory_kib
            && self.memory_kib <= 4 * 1024 * 1024
            && self.iterations >= 1
            && self.iterations <= 64
            && (1..=16).contains(&self.parallelism);
        if ok {
            Ok(())
        } else {
            Err(CoreError::InvalidInput {
                message: "unsupported Argon2id parameters".into(),
            })
        }
    }
}

/// Derives a 32-byte key from a password.
pub fn derive(
    password: &[u8],
    salt: &[u8; SALT_LEN],
    cost: Argon2Cost,
) -> CoreResult<Zeroizing<[u8; 32]>> {
    cost.validate()?;
    derive_unchecked(password, salt, cost)
}

fn derive_unchecked(
    password: &[u8],
    salt: &[u8],
    cost: Argon2Cost,
) -> CoreResult<Zeroizing<[u8; 32]>> {
    let params = Params::new(cost.memory_kib, cost.iterations, cost.parallelism, Some(32))
        .map_err(|_| CoreError::Crypto)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|_| CoreError::Crypto)?;
    Ok(out)
}

/// Times one derivation at `cost`, for calibrating to the device.
pub fn time_cost(cost: Argon2Cost) -> CoreResult<Duration> {
    let started = Instant::now();
    derive_unchecked(b"atropos-calibration", &[0u8; SALT_LEN], cost)?;
    Ok(started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Argon2Cost = Argon2Cost {
        memory_kib: 64 * 1024,
        iterations: 1,
        parallelism: 1,
    };

    #[test]
    fn deterministic_and_salted() {
        let a = derive(b"pw", &[1; 16], FAST).unwrap();
        let b = derive(b"pw", &[1; 16], FAST).unwrap();
        let c = derive(b"pw", &[2; 16], FAST).unwrap();
        let d = derive(b"pW", &[1; 16], FAST).unwrap();
        assert_eq!(*a, *b);
        assert_ne!(*a, *c);
        assert_ne!(*a, *d);
    }

    #[test]
    fn refuses_weak_or_hostile_costs() {
        let weak = Argon2Cost {
            memory_kib: 1024,
            ..FAST
        };
        assert!(derive(b"pw", &[1; 16], weak).is_err());
        let huge = Argon2Cost {
            memory_kib: u32::MAX,
            ..FAST
        };
        assert!(derive(b"pw", &[1; 16], huge).is_err());
    }

    /// RFC 9106 section 5.3 test vector (Argon2id, with secret and AD).
    #[test]
    fn rfc9106_vector() {
        let params_ad = argon2::ParamsBuilder::new()
            .m_cost(32)
            .t_cost(3)
            .p_cost(4)
            .output_len(32)
            .data(argon2::AssociatedData::new(&[4u8; 12]).unwrap())
            .build()
            .unwrap();
        let argon_ad =
            Argon2::new_with_secret(&[3u8; 8], Algorithm::Argon2id, Version::V0x13, params_ad)
                .unwrap();
        let mut out = [0u8; 32];
        argon_ad
            .hash_password_into(&[1u8; 32], &[2u8; 16], &mut out)
            .unwrap();
        let expected = [
            0x0d, 0x64, 0x0d, 0xf5, 0x8d, 0x78, 0x76, 0x6c, 0x08, 0xc0, 0x37, 0xa3, 0x4a, 0x8b,
            0x53, 0xc9, 0xd0, 0x1e, 0xf0, 0x45, 0x2d, 0x75, 0xb6, 0x5e, 0xb5, 0x25, 0x20, 0xe9,
            0x6b, 0x01, 0xe6, 0x59,
        ];
        assert_eq!(out, expected);
    }
}
