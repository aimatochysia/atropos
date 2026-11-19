//! Recovery keys: 256 random bits shown as 24 BIP-39 words.
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

pub fn to_phrase(key: &[u8; 32]) -> CoreResult<Zeroizing<String>> {
    let m = bip39::Mnemonic::from_entropy(key).map_err(|_| CoreError::Crypto)?;
    Ok(Zeroizing::new(m.words().collect::<Vec<_>>().join(" ")))
}

/// Accepts any spacing and capitalisation; the BIP-39 checksum catches typos.
pub fn from_phrase(phrase: &str) -> CoreResult<Zeroizing<[u8; 32]>> {
    let normal = Zeroizing::new(
        phrase
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let bad = || CoreError::InvalidInput {
        message: "that recovery phrase is not valid; check each of the 24 words".into(),
    };
    let m = bip39::Mnemonic::parse_normalized(&normal).map_err(|_| bad())?;
    if m.word_count() != 24 {
        return Err(bad());
    }
    let entropy = Zeroizing::new(m.to_entropy());
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&entropy);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_typo() {
        let key = [7u8; 32];
        let phrase = to_phrase(&key).unwrap();
        assert_eq!(phrase.split(' ').count(), 24);
        let messy = format!("  {}  ", phrase.to_uppercase().replace(' ', "   "));
        assert_eq!(*from_phrase(&messy).unwrap(), key);
        let mut words: Vec<&str> = phrase.split(' ').collect();
        words.swap(0, 1);
        assert!(from_phrase(&words.join(" ")).is_err() || words[0] == words[1]);
        assert!(from_phrase("abandon abandon").is_err());
    }
}
