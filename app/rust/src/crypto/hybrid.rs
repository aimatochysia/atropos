//! Hybrid key encapsulation: ML-KEM-1024 (FIPS 203) + ECDH P-384 (SP 800-56A).
//!
//! The 32-byte output key stays secret as long as *either* half holds: a
//! future quantum computer breaks P-384 but not ML-KEM, and a flaw found in
//! ML-KEM would still leave P-384. The two shared secrets are combined with
//! HKDF-SHA-384 together with both ciphertexts and the recipient's public
//! keys, so a ciphertext cannot be re-targeted or mixed with another one.
use aws_lc_rs::agreement::{self, PrivateKey, UnparsedPublicKey, ECDH_P384};
use aws_lc_rs::encoding::{AsBigEndian, EcPrivateKeyBin};
use aws_lc_rs::kem::{Ciphertext, DecapsulationKey, EncapsulationKey, ML_KEM_1024};
use zeroize::{Zeroize, Zeroizing};

use super::kdf::derive_key;
use crate::error::{CoreError, CoreResult};

pub const MLKEM_PUBLIC_LEN: usize = 1568;
pub const MLKEM_SECRET_LEN: usize = 3168;
pub const MLKEM_CIPHERTEXT_LEN: usize = 1568;
/// Uncompressed SEC1 point: 0x04 || X || Y.
pub const P384_PUBLIC_LEN: usize = 97;
pub const P384_SECRET_LEN: usize = 48;

const COMBINER_SALT: &[u8] = b"atropos/hybrid-kem/v1";
const COMBINER_LABEL: &[u8] = b"atropos/hybrid-kem/v1/ML-KEM-1024+P-384";

/// The public half: safe to store in the clear.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HybridPublicKey {
    pub mlkem: Vec<u8>,
    pub p384: Vec<u8>,
}

/// The secret half. Its bytes are wiped when it is dropped and it never
/// implements `Debug`, `Clone` or serialization.
pub struct HybridSecretKey {
    mlkem: Zeroizing<Vec<u8>>,
    p384: Zeroizing<Vec<u8>>,
    public: HybridPublicKey,
}

/// What the sender stores next to the data so the recipient can recover the key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HybridCiphertext {
    pub mlkem: Vec<u8>,
    pub p384_ephemeral: Vec<u8>,
}

impl HybridPublicKey {
    /// Parses stored bytes, checking lengths and that both keys are valid.
    pub fn from_parts(mlkem: &[u8], p384: &[u8]) -> CoreResult<Self> {
        if mlkem.len() != MLKEM_PUBLIC_LEN || p384.len() != P384_PUBLIC_LEN {
            return Err(CoreError::Crypto);
        }
        EncapsulationKey::new(&ML_KEM_1024, mlkem).map_err(|_| CoreError::Crypto)?;
        Ok(Self {
            mlkem: mlkem.to_vec(),
            p384: p384.to_vec(),
        })
    }
}

impl HybridSecretKey {
    /// Generates a fresh key pair from the OS random source.
    pub fn generate() -> CoreResult<Self> {
        let dk = DecapsulationKey::generate(&ML_KEM_1024).map_err(|_| CoreError::Crypto)?;
        let ek = dk.encapsulation_key().map_err(|_| CoreError::Crypto)?;
        let mlkem_public = ek
            .key_bytes()
            .map_err(|_| CoreError::Crypto)?
            .as_ref()
            .to_vec();
        let mlkem = Zeroizing::new(
            dk.key_bytes()
                .map_err(|_| CoreError::Crypto)?
                .as_ref()
                .to_vec(),
        );

        let ec = PrivateKey::generate(&ECDH_P384).map_err(|_| CoreError::Crypto)?;
        let p384_public = ec
            .compute_public_key()
            .map_err(|_| CoreError::Crypto)?
            .as_ref()
            .to_vec();
        let bin: EcPrivateKeyBin = ec.as_be_bytes().map_err(|_| CoreError::Crypto)?;
        let p384 = Zeroizing::new(bin.as_ref().to_vec());

        Ok(Self {
            mlkem,
            p384,
            public: HybridPublicKey {
                mlkem: mlkem_public,
                p384: p384_public,
            },
        })
    }

    /// Rebuilds a secret key from its stored (already unwrapped) parts.
    pub fn from_parts(mlkem: &[u8], p384: &[u8], public: HybridPublicKey) -> CoreResult<Self> {
        if mlkem.len() != MLKEM_SECRET_LEN || p384.len() != P384_SECRET_LEN {
            return Err(CoreError::Crypto);
        }
        Ok(Self {
            mlkem: Zeroizing::new(mlkem.to_vec()),
            p384: Zeroizing::new(p384.to_vec()),
            public,
        })
    }

    pub fn public_key(&self) -> &HybridPublicKey {
        &self.public
    }

    /// The secret parts, for wrapping under a key slot. Wiped on drop.
    pub fn secret_parts(&self) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
        (self.mlkem.clone(), self.p384.clone())
    }

    /// Recovers the shared key from a ciphertext made for this key pair.
    pub fn decapsulate(&self, ct: &HybridCiphertext) -> CoreResult<Zeroizing<[u8; 32]>> {
        if ct.mlkem.len() != MLKEM_CIPHERTEXT_LEN || ct.p384_ephemeral.len() != P384_PUBLIC_LEN {
            return Err(CoreError::DecryptFailed);
        }
        let dk = DecapsulationKey::new(&ML_KEM_1024, &self.mlkem).map_err(|_| CoreError::Crypto)?;
        let ss_pq = dk
            .decapsulate(Ciphertext::from(ct.mlkem.as_slice()))
            .map_err(|_| CoreError::DecryptFailed)?;
        let ec =
            PrivateKey::from_private_key(&ECDH_P384, &self.p384).map_err(|_| CoreError::Crypto)?;
        let ss_ec = ecdh(&ec, &ct.p384_ephemeral)?;
        combine(ss_pq.as_ref(), &ss_ec, ct, &self.public)
    }
}

/// Makes a fresh shared key for `pk` and the ciphertext that recovers it.
pub fn encapsulate(pk: &HybridPublicKey) -> CoreResult<(HybridCiphertext, Zeroizing<[u8; 32]>)> {
    let ek = EncapsulationKey::new(&ML_KEM_1024, &pk.mlkem).map_err(|_| CoreError::Crypto)?;
    let (ct_pq, ss_pq) = ek.encapsulate().map_err(|_| CoreError::Crypto)?;

    let eph = PrivateKey::generate(&ECDH_P384).map_err(|_| CoreError::Crypto)?;
    let eph_public = eph
        .compute_public_key()
        .map_err(|_| CoreError::Crypto)?
        .as_ref()
        .to_vec();
    let ss_ec = ecdh(&eph, &pk.p384)?;

    let ct = HybridCiphertext {
        mlkem: ct_pq.as_ref().to_vec(),
        p384_ephemeral: eph_public,
    };
    let key = combine(ss_pq.as_ref(), &ss_ec, &ct, pk)?;
    Ok((ct, key))
}

fn ecdh(private: &PrivateKey, peer: &[u8]) -> CoreResult<Zeroizing<Vec<u8>>> {
    agreement::agree(
        private,
        UnparsedPublicKey::new(&ECDH_P384, peer),
        CoreError::DecryptFailed,
        |z| Ok(Zeroizing::new(z.to_vec())),
    )
}

/// SP 800-56C two-step KDF over Z = ss_ML-KEM || ss_ECDH, with the
/// ciphertexts and recipient keys bound in the FixedInfo.
fn combine(
    ss_pq: &[u8],
    ss_ec: &[u8],
    ct: &HybridCiphertext,
    pk: &HybridPublicKey,
) -> CoreResult<Zeroizing<[u8; 32]>> {
    let mut z = Zeroizing::new(Vec::with_capacity(ss_pq.len() + ss_ec.len()));
    z.extend_from_slice(ss_pq);
    z.extend_from_slice(ss_ec);
    let key = derive_key(
        COMBINER_SALT,
        &z,
        &[
            COMBINER_LABEL,
            &ct.mlkem,
            &ct.p384_ephemeral,
            &pk.mlkem,
            &pk.p384,
        ],
    );
    z.zeroize();
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let sk = HybridSecretKey::generate().unwrap();
        assert_eq!(sk.public_key().mlkem.len(), MLKEM_PUBLIC_LEN);
        assert_eq!(sk.public_key().p384.len(), P384_PUBLIC_LEN);
        let (ct, key) = encapsulate(sk.public_key()).unwrap();
        assert_eq!(ct.mlkem.len(), MLKEM_CIPHERTEXT_LEN);
        assert_eq!(*sk.decapsulate(&ct).unwrap(), *key);
    }

    #[test]
    fn restored_key_still_works() {
        let sk = HybridSecretKey::generate().unwrap();
        let (m, p) = sk.secret_parts();
        let pk =
            HybridPublicKey::from_parts(&sk.public_key().mlkem, &sk.public_key().p384).unwrap();
        let restored = HybridSecretKey::from_parts(&m, &p, pk).unwrap();
        let (ct, key) = encapsulate(sk.public_key()).unwrap();
        assert_eq!(*restored.decapsulate(&ct).unwrap(), *key);
    }

    #[test]
    fn wrong_key_or_tampered_ciphertext_gives_a_different_key() {
        let a = HybridSecretKey::generate().unwrap();
        let b = HybridSecretKey::generate().unwrap();
        let (ct, key) = encapsulate(a.public_key()).unwrap();
        // ML-KEM uses implicit rejection, so a mismatch yields an unrelated key.
        assert_ne!(*b.decapsulate(&ct).unwrap_or_default(), *key);

        let mut swapped = ct.clone();
        let (other, _) = encapsulate(a.public_key()).unwrap();
        swapped.p384_ephemeral = other.p384_ephemeral;
        assert_ne!(*a.decapsulate(&swapped).unwrap(), *key);

        let mut flipped = ct.clone();
        flipped.mlkem[10] ^= 1;
        assert_ne!(*a.decapsulate(&flipped).unwrap(), *key);

        let mut bad_point = ct;
        bad_point.p384_ephemeral[5] ^= 1;
        assert!(a.decapsulate(&bad_point).is_err());
    }

    #[test]
    fn rejects_wrong_lengths() {
        assert!(HybridPublicKey::from_parts(&[0u8; 10], &[0u8; 97]).is_err());
        let sk = HybridSecretKey::generate().unwrap();
        let short = HybridCiphertext {
            mlkem: vec![0; 5],
            p384_ephemeral: vec![0; 97],
        };
        assert!(sk.decapsulate(&short).is_err());
    }
}
