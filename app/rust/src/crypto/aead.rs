//! AES-256-GCM with 96-bit nonces (SP 800-38D).
use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

pub const KEY_LEN: usize = 32;
pub const TAG_LEN: usize = 16;

/// A 256-bit AES-GCM key. The raw bytes are wiped on drop; aws-lc keeps its
/// own expanded copy, which it clears when `LessSafeKey` is dropped.
pub struct AeadKey(LessSafeKey);

impl AeadKey {
    pub fn new(key: &[u8; KEY_LEN]) -> CoreResult<Self> {
        let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| CoreError::Crypto)?;
        Ok(Self(LessSafeKey::new(unbound)))
    }

    /// Encrypts `plaintext` and returns ciphertext || tag. The caller must
    /// never use the same nonce twice with the same key.
    pub fn seal(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        plaintext: &[u8],
    ) -> CoreResult<Vec<u8>> {
        let mut buf = Vec::with_capacity(plaintext.len() + TAG_LEN);
        buf.extend_from_slice(plaintext);
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(*nonce),
                Aad::from(aad),
                &mut buf,
            )
            .map_err(|_| CoreError::Crypto)?;
        Ok(buf)
    }

    /// Decrypts ciphertext || tag. Fails without detail if the key, nonce,
    /// associated data or ciphertext do not match.
    pub fn open(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        sealed: &[u8],
    ) -> CoreResult<Zeroizing<Vec<u8>>> {
        if sealed.len() < TAG_LEN {
            return Err(CoreError::DecryptFailed);
        }
        let mut buf = Zeroizing::new(sealed.to_vec());
        let len = self
            .0
            .open_in_place(
                Nonce::assume_unique_for_key(*nonce),
                Aad::from(aad),
                &mut buf,
            )
            .map_err(|_| CoreError::DecryptFailed)?
            .len();
        buf.truncate(len);
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let key = AeadKey::new(&[7u8; 32]).unwrap();
        let nonce = [1u8; 12];
        let sealed = key.seal(&nonce, b"hdr", b"hello").unwrap();
        assert_eq!(sealed.len(), 5 + TAG_LEN);
        assert_eq!(&key.open(&nonce, b"hdr", &sealed).unwrap()[..], b"hello");

        let mut bad = sealed.clone();
        bad[0] ^= 1;
        assert!(matches!(
            key.open(&nonce, b"hdr", &bad),
            Err(CoreError::DecryptFailed)
        ));
        assert!(key.open(&nonce, b"other", &sealed).is_err());
        assert!(key.open(&[2u8; 12], b"hdr", &sealed).is_err());
        assert!(key.open(&nonce, b"hdr", &sealed[..4]).is_err());
    }

    /// NIST SP 800-38D test case 16 (AES-256, 96-bit IV, 60-byte plaintext, AAD).
    #[test]
    fn known_answer() {
        let hex = |s: &str| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>()
        };
        let key: [u8; 32] = hex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308")
            .try_into()
            .unwrap();
        let nonce: [u8; 12] = hex("cafebabefacedbaddecaf888").try_into().unwrap();
        let pt = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39");
        let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let expected = hex(concat!(
            "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa",
            "8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
            "76fc6ece0f4e1768cddf8853bb2d551b"
        ));
        let key = AeadKey::new(&key).unwrap();
        let out = key.seal(&nonce, &aad, &pt).unwrap();
        assert_eq!(out, expected);
        assert_eq!(&key.open(&nonce, &aad, &out).unwrap()[..], &pt[..]);
    }
}
