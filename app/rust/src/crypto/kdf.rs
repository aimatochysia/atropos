//! HKDF-SHA-384 (SP 800-56C r2, two-step) with a distinct label per key role.
use aws_lc_rs::hkdf::{KeyType, Salt, HKDF_SHA384};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// Derives a 32-byte key from `ikm`. `info` parts are concatenated as the
/// HKDF info; every caller passes its own `atropos/...` label first.
pub fn derive_key(salt: &[u8], ikm: &[u8], info: &[&[u8]]) -> CoreResult<Zeroizing<[u8; 32]>> {
    let prk = Salt::new(HKDF_SHA384, salt).extract(ikm);
    let okm = prk.expand(info, Len(32)).map_err(|_| CoreError::Crypto)?;
    let mut out = Zeroizing::new([0u8; 32]);
    okm.fill(out.as_mut()).map_err(|_| CoreError::Crypto)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_separate_keys() {
        let a = derive_key(b"s", b"ikm", &[b"atropos/a"]).unwrap();
        let b = derive_key(b"s", b"ikm", &[b"atropos/b"]).unwrap();
        let a2 = derive_key(b"s", b"ikm", &[b"atropos/", b"a"]).unwrap();
        assert_ne!(*a, *b);
        assert_eq!(*a, *a2);
    }
}
