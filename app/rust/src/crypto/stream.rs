//! Chunked AES-256-GCM for files of any size (the STREAM construction of
//! Hoang, Reyhanitabar, Rogaway and Vizár).
//!
//! Each file gets its own random key, so nonces only need to be unique within
//! one file: nonce = 7-byte random prefix || 4-byte chunk counter || last flag.
//! Reordering, dropping, duplicating or truncating chunks, or moving a chunk
//! to another file, all fail authentication.
use std::io::{Read, Write};

use zeroize::Zeroizing;

use super::aead::{AeadKey, TAG_LEN};
use crate::error::{CoreError, CoreResult};

/// Plaintext bytes per chunk.
pub const CHUNK_LEN: usize = 64 * 1024;
pub const PREFIX_LEN: usize = 7;

fn nonce(prefix: &[u8; PREFIX_LEN], counter: u32, last: bool) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..PREFIX_LEN].copy_from_slice(prefix);
    n[PREFIX_LEN..11].copy_from_slice(&counter.to_be_bytes());
    n[11] = u8::from(last);
    n
}

/// Reads up to `buf.len()` bytes, stopping early only at end of input.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> CoreResult<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(filled)
}

/// Encrypts everything from `input` into `output`. `aad` binds the stream to
/// its context (for example the file id). Returns the plaintext length.
pub fn encrypt(
    key: &[u8; 32],
    prefix: &[u8; PREFIX_LEN],
    aad: &[u8],
    input: &mut impl Read,
    output: &mut impl Write,
) -> CoreResult<u64> {
    let key = AeadKey::new(key)?;
    let mut current = Zeroizing::new(vec![0u8; CHUNK_LEN]);
    let mut next = Zeroizing::new(vec![0u8; CHUNK_LEN]);
    let mut len = read_full(input, &mut current)?;
    let mut counter: u32 = 0;
    let mut total = 0u64;
    loop {
        // Look ahead one chunk so the last chunk can be flagged.
        let next_len = if len == CHUNK_LEN {
            read_full(input, &mut next)?
        } else {
            0
        };
        let last = next_len == 0;
        let sealed = key.seal(&nonce(prefix, counter, last), aad, &current[..len])?;
        output.write_all(&sealed)?;
        total += len as u64;
        if last {
            break;
        }
        counter = counter.checked_add(1).ok_or(CoreError::InvalidInput {
            message: "file is too large".into(),
        })?;
        std::mem::swap(&mut current, &mut next);
        len = next_len;
    }
    output.flush()?;
    Ok(total)
}

/// Decrypts a stream made by [`encrypt`]. Plaintext is written chunk by chunk
/// only after each chunk authenticates; on error the caller must discard what
/// was written so far.
pub fn decrypt(
    key: &[u8; 32],
    prefix: &[u8; PREFIX_LEN],
    aad: &[u8],
    input: &mut impl Read,
    output: &mut impl Write,
) -> CoreResult<u64> {
    let key = AeadKey::new(key)?;
    let sealed_len = CHUNK_LEN + TAG_LEN;
    let mut current = vec![0u8; sealed_len];
    let mut next = vec![0u8; sealed_len];
    let mut len = read_full(input, &mut current)?;
    let mut counter: u32 = 0;
    let mut total = 0u64;
    loop {
        if len < TAG_LEN {
            return Err(CoreError::DecryptFailed);
        }
        let next_len = if len == sealed_len {
            read_full(input, &mut next)?
        } else {
            0
        };
        let last = next_len == 0;
        let plain = key.open(&nonce(prefix, counter, last), aad, &current[..len])?;
        output.write_all(&plain)?;
        total += plain.len() as u64;
        if last {
            break;
        }
        counter = counter.checked_add(1).ok_or(CoreError::DecryptFailed)?;
        std::mem::swap(&mut current, &mut next);
        len = next_len;
    }
    output.flush()?;
    Ok(total)
}

/// Size of the encrypted stream for a plaintext of `len` bytes.
pub fn sealed_len(len: u64) -> u64 {
    let chunks = if len == 0 {
        1
    } else {
        len.div_ceil(CHUNK_LEN as u64)
    };
    len + chunks * TAG_LEN as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encrypt(&[9; 32], &[1; 7], b"file-1", &mut &data[..], &mut out).unwrap();
        out
    }

    fn dec(data: &[u8], aad: &[u8]) -> CoreResult<Vec<u8>> {
        let mut out = Vec::new();
        decrypt(&[9; 32], &[1; 7], aad, &mut &data[..], &mut out)?;
        Ok(out)
    }

    #[test]
    fn round_trips_at_chunk_boundaries() {
        for len in [
            0,
            1,
            CHUNK_LEN - 1,
            CHUNK_LEN,
            CHUNK_LEN + 1,
            3 * CHUNK_LEN,
            3 * CHUNK_LEN + 7,
        ] {
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let sealed = enc(&data);
            assert_eq!(sealed.len() as u64, sealed_len(len as u64), "len {len}");
            assert_eq!(dec(&sealed, b"file-1").unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn detects_truncation_reordering_and_context_swap() {
        let data = vec![5u8; 2 * CHUNK_LEN + 10];
        let sealed = enc(&data);
        let c = CHUNK_LEN + TAG_LEN;
        // Dropping the final chunk: the new last chunk was not flagged as last.
        assert!(dec(&sealed[..2 * c], b"file-1").is_err());
        // Swapping the first two chunks.
        let mut swapped = sealed[c..2 * c].to_vec();
        swapped.extend_from_slice(&sealed[..c]);
        swapped.extend_from_slice(&sealed[2 * c..]);
        assert!(dec(&swapped, b"file-1").is_err());
        // Same bytes presented as another file.
        assert!(dec(&sealed, b"file-2").is_err());
        // Extra bytes appended.
        let mut longer = sealed.clone();
        longer.extend_from_slice(&[0u8; 20]);
        assert!(dec(&longer, b"file-1").is_err());
    }
}
