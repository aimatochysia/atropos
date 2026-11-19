//! Blobs: the encrypted contents of files, in `objects/<xx>/<id>.blob`.
//!
//! ```text
//! "ATB1" | ML-KEM-1024 ciphertext (1568) | P-384 ephemeral point (97) | STREAM prefix (7)
//! | STREAM(AES-256-GCM, 64 KiB chunks) of the padded plaintext
//! ```
//!
//! Each blob's key comes from a fresh hybrid encapsulation to the vault's
//! public key, so only the vault's private key can open it, and new blobs can
//! be written knowing only the public key. A blob holds either one large file
//! or a pack of small files; entries point at (blob, offset, length). The
//! plaintext is zero-padded to a Padmé length so sizes leak little.
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::manifest::Id;
use crate::crypto::aead::{AeadKey, TAG_LEN};
use crate::crypto::hybrid::{
    self, HybridCiphertext, HybridPublicKey, HybridSecretKey, MLKEM_CIPHERTEXT_LEN, P384_PUBLIC_LEN,
};
use crate::crypto::stream::{self, CHUNK_LEN, PREFIX_LEN};
use crate::crypto::{padme, random_array};
use crate::error::{CoreError, CoreResult};

const MAGIC: &[u8; 4] = b"ATB1";
pub const HEADER_LEN: usize = 4 + MLKEM_CIPHERTEXT_LEN + P384_PUBLIC_LEN + PREFIX_LEN;
/// Smallest padded blob, so tiny files do not stand out.
const MIN_PADDED: u64 = 4096;

pub fn padded_len(len: u64) -> u64 {
    padme(len.max(MIN_PADDED))
}

pub fn blob_path(objects: &Path, id: &Id) -> PathBuf {
    let h = hex::encode(id);
    objects.join(&h[..2]).join(format!("{h}.blob"))
}

fn aad(vault_id: &[u8], id: &Id) -> Vec<u8> {
    let mut a = b"atropos/blob/v1".to_vec();
    a.extend_from_slice(vault_id);
    a.extend_from_slice(id);
    a
}

/// Reads `inner`, then zeros up to `total` bytes. Fails if `inner` yields
/// a different number of bytes than `expected` (the file changed meanwhile).
struct PadReader<R> {
    inner: R,
    expected: u64,
    total: u64,
    read: u64,
}

impl<R: Read> Read for PadReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.read < self.expected {
            let want = buf.len().min((self.expected - self.read) as usize);
            let n = self.inner.read(&mut buf[..want])?;
            if n == 0 {
                return Err(std::io::Error::other(
                    "the file got shorter while it was being imported",
                ));
            }
            self.read += n as u64;
            return Ok(n);
        }
        if self.read == self.expected && self.expected > 0 {
            let mut probe = [0u8; 1];
            if self.inner.read(&mut probe)? != 0 {
                return Err(std::io::Error::other(
                    "the file grew while it was being imported",
                ));
            }
        }
        let want = buf.len().min((self.total - self.read) as usize);
        buf[..want].fill(0);
        self.read += want as u64;
        Ok(want)
    }
}

/// Encrypts `len` bytes from `input` into a new blob. Returns the blob id.
pub fn write_blob(
    objects: &Path,
    vault_id: &[u8],
    public: &HybridPublicKey,
    input: impl Read,
    len: u64,
) -> CoreResult<Id> {
    let id = random_array::<16>()?;
    let (ct, key) = hybrid::encapsulate(public)?;
    let prefix = random_array::<PREFIX_LEN>()?;
    let path = blob_path(objects, &id);
    fs::create_dir_all(path.parent().ok_or(CoreError::Crypto)?)?;
    let tmp = path.with_extension("tmp");
    let result = (|| -> CoreResult<()> {
        let mut out = std::io::BufWriter::new(File::create(&tmp)?);
        out.write_all(MAGIC)?;
        out.write_all(&ct.mlkem)?;
        out.write_all(&ct.p384_ephemeral)?;
        out.write_all(&prefix)?;
        let mut reader = PadReader {
            inner: input,
            expected: len,
            total: padded_len(len),
            read: 0,
        };
        stream::encrypt(&key, &prefix, &aad(vault_id, &id), &mut reader, &mut out)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, &path)?;
    Ok(id)
}

/// Opens blobs of an unlocked vault, caching each blob's key.
pub struct BlobReader {
    objects: PathBuf,
    vault_id: Vec<u8>,
    keys: HashMap<Id, (Zeroizing<[u8; 32]>, [u8; PREFIX_LEN])>,
}

impl BlobReader {
    pub fn new(objects: &Path, vault_id: &[u8]) -> Self {
        Self {
            objects: objects.to_path_buf(),
            vault_id: vault_id.to_vec(),
            keys: HashMap::new(),
        }
    }

    pub fn forget_keys(&mut self) {
        self.keys.clear();
    }

    fn key(
        &mut self,
        secret: &HybridSecretKey,
        id: &Id,
        file: &mut File,
    ) -> CoreResult<(AeadKey, [u8; PREFIX_LEN])> {
        if let Some((k, p)) = self.keys.get(id) {
            return Ok((AeadKey::new(k)?, *p));
        }
        let mut head = [0u8; HEADER_LEN];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut head)
            .map_err(|_| CoreError::DecryptFailed)?;
        if &head[..4] != MAGIC {
            return Err(CoreError::DecryptFailed);
        }
        let ct = HybridCiphertext {
            mlkem: head[4..4 + MLKEM_CIPHERTEXT_LEN].to_vec(),
            p384_ephemeral: head
                [4 + MLKEM_CIPHERTEXT_LEN..4 + MLKEM_CIPHERTEXT_LEN + P384_PUBLIC_LEN]
                .to_vec(),
        };
        let mut prefix = [0u8; PREFIX_LEN];
        prefix.copy_from_slice(&head[HEADER_LEN - PREFIX_LEN..]);
        let key = secret.decapsulate(&ct)?;
        let aead = AeadKey::new(&key)?;
        self.keys.insert(*id, (key, prefix));
        Ok((aead, prefix))
    }

    /// Streams plaintext bytes `offset..offset+len` of blob `id` into `out`,
    /// authenticating every chunk it touches.
    pub fn read_range(
        &mut self,
        secret: &HybridSecretKey,
        id: &Id,
        offset: u64,
        len: u64,
        out: &mut impl Write,
    ) -> CoreResult<()> {
        let path = blob_path(&self.objects, id);
        let mut file = File::open(&path).map_err(|_| CoreError::InvalidInput {
            message: "part of this vault is missing (a blob file was deleted or not synced yet)"
                .into(),
        })?;
        let (key, prefix) = self.key(secret, id, &mut file)?;
        let aad = aad(&self.vault_id, id);
        let sealed_total = file.metadata()?.len().saturating_sub(HEADER_LEN as u64);
        let chunks = stream::chunk_count(sealed_total);
        let sealed_chunk = (CHUNK_LEN + TAG_LEN) as u64;
        if len == 0 {
            return Ok(());
        }
        let first = offset / CHUNK_LEN as u64;
        let last_byte = offset + len - 1;
        let last = last_byte / CHUNK_LEN as u64;
        if last >= chunks {
            return Err(CoreError::DecryptFailed);
        }
        let mut buf = vec![0u8; CHUNK_LEN + TAG_LEN];
        for i in first..=last {
            let start = HEADER_LEN as u64 + i * sealed_chunk;
            let size = if i + 1 == chunks {
                (sealed_total - i * sealed_chunk) as usize
            } else {
                sealed_chunk as usize
            };
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut buf[..size])
                .map_err(|_| CoreError::DecryptFailed)?;
            let plain =
                stream::decrypt_chunk(&key, &prefix, &aad, i, i + 1 == chunks, &buf[..size])?;
            let chunk_start = i * CHUNK_LEN as u64;
            let from = offset.saturating_sub(chunk_start) as usize;
            let to = ((last_byte - chunk_start) as usize + 1).min(plain.len());
            if from > to {
                return Err(CoreError::DecryptFailed);
            }
            out.write_all(&plain[from..to])?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &Id) {
        self.keys.remove(id);
        let _ = fs::remove_file(blob_path(&self.objects, id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let sk = HybridSecretKey::generate().unwrap();
        let data: Vec<u8> = (0..(3 * CHUNK_LEN + 500))
            .map(|i| (i % 253) as u8)
            .collect();
        let id = write_blob(
            dir.path(),
            b"v",
            sk.public_key(),
            &data[..],
            data.len() as u64,
        )
        .unwrap();
        let size = fs::metadata(blob_path(dir.path(), &id)).unwrap().len();
        assert!(size >= padded_len(data.len() as u64));
        let mut r = BlobReader::new(dir.path(), b"v");
        for (off, len) in [
            (0u64, 10u64),
            (CHUNK_LEN as u64 - 5, 10),
            (0, data.len() as u64),
            (3 * CHUNK_LEN as u64 + 499, 1),
        ] {
            let mut out = Vec::new();
            r.read_range(&sk, &id, off, len, &mut out).unwrap();
            assert_eq!(out, &data[off as usize..(off + len) as usize]);
        }
        // Another vault's key cannot read it.
        let other = HybridSecretKey::generate().unwrap();
        let mut r2 = BlobReader::new(dir.path(), b"v");
        assert!(r2.read_range(&other, &id, 0, 10, &mut Vec::new()).is_err());
        // Nor can the same key under another vault id.
        let mut r3 = BlobReader::new(dir.path(), b"w");
        assert!(r3.read_range(&sk, &id, 0, 10, &mut Vec::new()).is_err());
    }

    #[test]
    fn size_mismatch_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let sk = HybridSecretKey::generate().unwrap();
        assert!(write_blob(dir.path(), b"v", sk.public_key(), &[1u8; 10][..], 20).is_err());
        assert!(write_blob(dir.path(), b"v", sk.public_key(), &[1u8; 30][..], 20).is_err());
        // No temp files left behind.
        let leftovers: Vec<_> = walk(dir.path());
        assert!(leftovers
            .iter()
            .all(|p| p.extension().is_none_or(|e| e != "tmp")));
    }

    fn walk(p: &Path) -> Vec<PathBuf> {
        let mut v = Vec::new();
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap().path();
            if e.is_dir() {
                v.extend(walk(&e));
            } else {
                v.push(e);
            }
        }
        v
    }
}
