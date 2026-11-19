//! Secret text typed in the app (passwords, recovery phrases) kept in Rust.
//!
//! The Atropos keypad sends each tap here as a character, so the secret never
//! exists as a Dart string and never passes through the system keyboard. The
//! buffer is wiped when the object is dropped or cleared.
use std::sync::Mutex;

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

#[frb(opaque)]
pub struct SecretInput {
    chars: Mutex<Zeroizing<Vec<char>>>,
}

/// How strong a password is, from zxcvbn (0 = very weak, 4 = very strong).
pub struct Strength {
    pub score: u8,
    pub acceptable: bool,
    pub warning: String,
    pub suggestion: String,
}

impl SecretInput {
    #[frb(sync)]
    pub fn new() -> SecretInput {
        SecretInput {
            chars: Mutex::new(Zeroizing::new(Vec::new())),
        }
    }

    /// For the ordinary text field fallback: the Dart string already exists,
    /// so nothing is gained or lost by copying it here.
    #[frb(sync)]
    pub fn from_text(text: String) -> SecretInput {
        let text = Zeroizing::new(text);
        SecretInput {
            chars: Mutex::new(Zeroizing::new(text.chars().collect())),
        }
    }

    #[frb(sync)]
    pub fn push(&self, codepoint: u32) {
        if let Some(c) = char::from_u32(codepoint) {
            if !c.is_control() {
                self.lock().push(c);
            }
        }
    }

    #[frb(sync)]
    pub fn pop(&self) {
        self.lock().pop();
    }

    #[frb(sync)]
    pub fn clear(&self) {
        let mut g = self.lock();
        g.iter_mut().for_each(|c| *c = '\0');
        g.clear();
    }

    #[frb(sync)]
    pub fn len(&self) -> u32 {
        self.lock().len() as u32
    }

    #[frb(sync)]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    #[frb(sync)]
    pub fn strength(&self) -> Strength {
        strength(&self.bytes())
    }

    #[frb(sync)]
    pub fn matches(&self, other: &SecretInput) -> bool {
        use subtle::ConstantTimeEq;
        let a = self.bytes();
        let b = other.bytes();
        a.len() == b.len() && bool::from(a.ct_eq(&b))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Zeroizing<Vec<char>>> {
        self.chars.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The secret as UTF-8 bytes, wiped when dropped.
    pub(crate) fn bytes(&self) -> Zeroizing<Vec<u8>> {
        let g = self.lock();
        let mut out = Zeroizing::new(Vec::with_capacity(g.len() * 2));
        let mut buf = [0u8; 4];
        for c in g.iter() {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        buf.iter_mut().for_each(|b| *b = 0);
        out
    }

    pub(crate) fn text(&self) -> Zeroizing<String> {
        Zeroizing::new(self.lock().iter().collect())
    }
}

impl Default for SecretInput {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimum accepted: zxcvbn score 3 and at least 10 characters.
pub(crate) fn strength(bytes: &[u8]) -> Strength {
    let text = Zeroizing::new(String::from_utf8_lossy(bytes).to_string());
    let len = text.chars().count();
    if len == 0 {
        return Strength {
            score: 0,
            acceptable: false,
            warning: String::new(),
            suggestion: "Use at least 10 characters.".into(),
        };
    }
    let e = zxcvbn::zxcvbn(&text, &["atropos", "vault", "password"]);
    let score = u8::from(e.score());
    let (warning, suggestion) = match e.feedback() {
        Some(f) => (
            f.warning().map(|w| w.to_string()).unwrap_or_default(),
            f.suggestions()
                .first()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    };
    let long_enough = len >= 10;
    Strength {
        score,
        acceptable: score >= 3 && long_enough,
        warning,
        suggestion: if long_enough {
            suggestion
        } else {
            "Use at least 10 characters.".into()
        },
    }
}

pub(crate) fn check_strength(bytes: &[u8]) -> CoreResult<()> {
    let s = strength(bytes);
    if s.acceptable {
        Ok(())
    } else {
        Err(CoreError::InvalidInput {
            message: format!(
                "that password is too easy to guess. {}",
                if s.suggestion.is_empty() {
                    "Add more words or characters."
                } else {
                    &s.suggestion
                }
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_strength() {
        let s = SecretInput::new();
        for c in "password1".chars() {
            s.push(c as u32);
        }
        assert_eq!(s.len(), 9);
        assert!(!s.strength().acceptable);
        s.clear();
        for c in "violet-harbor-tundra-9".chars() {
            s.push(c as u32);
        }
        s.push(0x7f); // control characters are ignored
        assert_eq!(s.len(), 22);
        assert!(s.strength().acceptable);
        assert!(s.matches(&SecretInput::from_text("violet-harbor-tundra-9".into())));
        s.pop();
        assert!(!s.matches(&SecretInput::from_text("violet-harbor-tundra-9".into())));
    }
}
