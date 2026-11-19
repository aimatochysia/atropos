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

/// Push-style encryption: write plaintext in, sealed chunks come out of
/// `inner`. Call [`StreamWriter::finish`] to seal the final chunk; dropping
/// the writer without it leaves a stream that will fail to decrypt.
pub struct StreamWriter<W: Write> {
    inner: W,
    key: AeadKey,
    prefix: [u8; PREFIX_LEN],
    aad: Vec<u8>,
    buf: Zeroizing<Vec<u8>>,
    counter: u32,
    pub written: u64,
}

impl<W: Write> StreamWriter<W> {
    pub fn new(inner: W, key: &[u8; 32], prefix: [u8; PREFIX_LEN], aad: &[u8]) -> CoreResult<Self> {
        Ok(Self {
            inner,
            key: AeadKey::new(key)?,
            prefix,
            aad: aad.to_vec(),
            buf: Zeroizing::new(Vec::with_capacity(CHUNK_LEN)),
            counter: 0,
            written: 0,
        })
    }

    fn seal(&mut self, last: bool) -> CoreResult<()> {
        let sealed = self.key.seal(
            &nonce(&self.prefix, self.counter, last),
            &self.aad,
            &self.buf,
        )?;
        self.inner.write_all(&sealed)?;
        self.buf.clear();
        self.counter = self.counter.checked_add(1).ok_or(CoreError::InvalidInput {
            message: "too much data for one stream".into(),
        })?;
        Ok(())
    }

    pub fn finish(mut self) -> CoreResult<W> {
        self.seal(true)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for StreamWriter<W> {
    fn write(&mut self, mut data: &[u8]) -> std::io::Result<usize> {
        let total = data.len();
        while !data.is_empty() {
            if self.buf.len() == CHUNK_LEN {
                // More data follows, so the full chunk is not the last one.
                self.seal(false).map_err(std::io::Error::other)?;
            }
            let take = (CHUNK_LEN - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        self.written += total as u64;
        Ok(total)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Pull-style decryption: reading yields authenticated plaintext. Hitting the
/// end of the stream without its final chunk is an error, so truncation is
/// always caught.
pub struct StreamReader<R: Read> {
    inner: R,
    key: AeadKey,
    prefix: [u8; PREFIX_LEN],
    aad: Vec<u8>,
    counter: u32,
    plain: Zeroizing<Vec<u8>>,
    pos: usize,
    peeked: Option<u8>,
    done: bool,
}

impl<R: Read> StreamReader<R> {
    pub fn new(inner: R, key: &[u8; 32], prefix: [u8; PREFIX_LEN], aad: &[u8]) -> CoreResult<Self> {
        Ok(Self {
            inner,
            key: AeadKey::new(key)?,
            prefix,
            aad: aad.to_vec(),
            counter: 0,
            plain: Zeroizing::new(Vec::new()),
            pos: 0,
            peeked: None,
            done: false,
        })
    }

    fn next_chunk(&mut self) -> CoreResult<()> {
        let mut sealed = vec![0u8; CHUNK_LEN + TAG_LEN];
        let mut filled = 0;
        if let Some(b) = self.peeked.take() {
            sealed[0] = b;
            filled = 1;
        }
        filled += read_full(&mut self.inner, &mut sealed[filled..])?;
        let mut last = filled < sealed.len();
        if !last {
            let mut one = [0u8; 1];
            if read_full(&mut self.inner, &mut one)? == 0 {
                last = true;
            } else {
                self.peeked = Some(one[0]);
            }
        }
        if filled < TAG_LEN {
            return Err(CoreError::DecryptFailed);
        }
        self.plain = self.key.open(
            &nonce(&self.prefix, self.counter, last),
            &self.aad,
            &sealed[..filled],
        )?;
        self.pos = 0;
        self.done = last;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(CoreError::DecryptFailed)?;
        Ok(())
    }

    /// True once the final chunk was authenticated and fully read.
    pub fn at_end(&mut self) -> CoreResult<bool> {
        while self.pos == self.plain.len() && !self.done {
            self.next_chunk()?;
        }
        Ok(self.done && self.pos == self.plain.len())
    }
}

impl<R: Read> Read for StreamReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos == self.plain.len() {
            if self.done {
                return Ok(0);
            }
            self.next_chunk().map_err(std::io::Error::other)?;
        }
        let n = buf.len().min(self.plain.len() - self.pos);
        buf[..n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Number of sealed chunks in a stream of `sealed` bytes.
pub fn chunk_count(sealed: u64) -> u64 {
    sealed.div_ceil((CHUNK_LEN + TAG_LEN) as u64).max(1)
}

/// Decrypts one sealed chunk for random access. `last` must be true only for
/// the final chunk of the stream (see [`chunk_count`]).
pub fn decrypt_chunk(
    key: &AeadKey,
    prefix: &[u8; PREFIX_LEN],
    aad: &[u8],
    index: u64,
    last: bool,
    sealed: &[u8],
) -> CoreResult<Zeroizing<Vec<u8>>> {
    let index = u32::try_from(index).map_err(|_| CoreError::DecryptFailed)?;
    key.open(&nonce(prefix, index, last), aad, sealed)
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

    #[test]
    fn writer_and_reader_match_the_pull_api() {
        for len in [0, 5, CHUNK_LEN, CHUNK_LEN + 3, 2 * CHUNK_LEN] {
            let data: Vec<u8> = (0..len).map(|i| (i % 13) as u8).collect();
            let mut w = StreamWriter::new(Vec::new(), &[9; 32], [1; 7], b"file-1").unwrap();
            for piece in data.chunks(1000) {
                w.write_all(piece).unwrap();
            }
            let pushed = w.finish().unwrap();
            assert_eq!(pushed, enc(&data), "len {len}");
            let mut r = StreamReader::new(&pushed[..], &[9; 32], [1; 7], b"file-1").unwrap();
            let mut back = Vec::new();
            r.read_to_end(&mut back).unwrap();
            assert_eq!(back, data);
            assert!(r.at_end().unwrap());
            if len > CHUNK_LEN {
                let cut = &pushed[..CHUNK_LEN + TAG_LEN];
                let mut r = StreamReader::new(cut, &[9; 32], [1; 7], b"file-1").unwrap();
                assert!(r.read_to_end(&mut Vec::new()).is_err());
            }
        }
    }

    #[test]
    fn random_access_matches_sequential() {
        let data: Vec<u8> = (0..(2 * CHUNK_LEN + 99)).map(|i| (i % 7) as u8).collect();
        let sealed = enc(&data);
        let n = chunk_count(sealed.len() as u64);
        assert_eq!(n, 3);
        let key = AeadKey::new(&[9; 32]).unwrap();
        let c = CHUNK_LEN + TAG_LEN;
        let last = decrypt_chunk(&key, &[1; 7], b"file-1", 2, true, &sealed[2 * c..]).unwrap();
        assert_eq!(&last[..], &data[2 * CHUNK_LEN..]);
        let first = decrypt_chunk(&key, &[1; 7], b"file-1", 0, false, &sealed[..c]).unwrap();
        assert_eq!(&first[..], &data[..CHUNK_LEN]);
        assert!(decrypt_chunk(&key, &[1; 7], b"file-1", 0, true, &sealed[..c]).is_err());
    }
}
