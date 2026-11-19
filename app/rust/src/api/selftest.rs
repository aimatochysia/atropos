//! A self-test the app runs on demand, so every platform build proves its
//! cryptography works on the real device and shows how fast it is.
use std::time::Instant;

use crate::crypto::{aead::AeadKey, hybrid, password, random_array, stream};
use crate::error::CoreResult;

/// One check in the self-test.
pub struct SelfTestStep {
    pub name: String,
    pub passed: bool,
    pub millis: u64,
    pub detail: String,
}

pub struct SelfTestReport {
    pub passed: bool,
    pub steps: Vec<SelfTestStep>,
}

fn step(name: &str, f: impl FnOnce() -> CoreResult<String>) -> SelfTestStep {
    let started = Instant::now();
    let result = f();
    let millis = started.elapsed().as_millis() as u64;
    match result {
        Ok(detail) => SelfTestStep {
            name: name.into(),
            passed: true,
            millis,
            detail,
        },
        Err(e) => SelfTestStep {
            name: name.into(),
            passed: false,
            millis,
            detail: e.to_string(),
        },
    }
}

/// Runs every primitive once: hybrid ML-KEM-1024 + P-384 round trip, a
/// tamper check, 4 MiB of chunked AES-256-GCM, and Argon2id at the default
/// 256 MiB cost.
pub fn run_crypto_self_test() -> SelfTestReport {
    let steps = vec![
        step("Hybrid KEM (ML-KEM-1024 + ECDH P-384)", || {
            let sk = hybrid::HybridSecretKey::generate()?;
            let (ct, key) = hybrid::encapsulate(sk.public_key())?;
            let back = sk.decapsulate(&ct)?;
            if *back != *key {
                return Err(crate::error::CoreError::Crypto);
            }
            Ok(format!(
                "public key {} B, ciphertext {} B",
                sk.public_key().mlkem.len() + sk.public_key().p384.len(),
                ct.mlkem.len() + ct.p384_ephemeral.len()
            ))
        }),
        step("AES-256-GCM rejects tampering", || {
            let key = AeadKey::new(&random_array::<32>()?)?;
            let nonce = random_array::<12>()?;
            let mut sealed = key.seal(&nonce, b"atropos", b"self-test")?;
            sealed[0] ^= 1;
            match key.open(&nonce, b"atropos", &sealed) {
                Err(_) => Ok("tampered data refused".into()),
                Ok(_) => Err(crate::error::CoreError::Crypto),
            }
        }),
        step("Chunked AES-256-GCM, 4 MiB", || {
            let key = random_array::<32>()?;
            let prefix = random_array::<7>()?;
            let data = vec![0x5au8; 4 * 1024 * 1024];
            let mut sealed = Vec::with_capacity(data.len() + 1024);
            let started = Instant::now();
            stream::encrypt(&key, &prefix, b"self-test", &mut &data[..], &mut sealed)?;
            let mut back = Vec::with_capacity(data.len());
            stream::decrypt(&key, &prefix, b"self-test", &mut &sealed[..], &mut back)?;
            if back != data {
                return Err(crate::error::CoreError::Crypto);
            }
            let secs = started.elapsed().as_secs_f64().max(1e-6);
            Ok(format!("{:.0} MiB/s encrypt + decrypt", 8.0 / secs))
        }),
        step("Argon2id, 256 MiB, 3 passes", || {
            let cost = password::Argon2Cost::DEFAULT;
            let took = password::time_cost(cost)?;
            Ok(format!("one unlock attempt takes {} ms", took.as_millis()))
        }),
    ];
    SelfTestReport {
        passed: steps.iter().all(|s| s.passed),
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes() {
        let report = run_crypto_self_test();
        for s in &report.steps {
            assert!(s.passed, "{}: {}", s.name, s.detail);
        }
        assert!(report.passed);
    }
}
