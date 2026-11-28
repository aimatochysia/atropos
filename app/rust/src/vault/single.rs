//! Single-file vaults: a whole vault in one `.atropos` file, with the same
//! key slots (password, recovery phrase, fingerprint, cards) as a folder
//! vault ("Vault LFS").
//!
//! ```text
//! "ATROPOS-VLT" 0x01
//! | key area A (64 KiB) | key area B (64 KiB)
//! | stream header length (u32 BE) | stream header JSON {prefix}
//! | STREAM(AES-256-GCM, 64 KiB chunks)
//!     plaintext: file bytes | index JSON | zero padding
//!                | index offset (u64 BE) | index length (u64 BE)
//! ```
//!
//! Each key area holds `length (u32 BE) | sequence (u64 BE) | header JSON`,
//! the same header (slots and MAC) a folder vault keeps in `vault.hdr`. A
//! header change (a new password, a fingerprint renewed) writes area A, then
//! area B, in place, so it never rewrites the files; if one write is cut
//! off, the other area still opens. The stream key comes from the vault's
//! root secret, and the stream's associated data binds the vault id and
//! public key, so the files cannot be moved under another vault's header.
//! Any change to the files writes a complete new copy beside the old one
//! and swaps it in.
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::header::{HeaderFile, VaultSecrets};
use super::manifest::{DataRef, Entry, Id, Kind};
use super::Progress;
use crate::crypto::aead::{AeadKey, TAG_LEN};
use crate::crypto::stream::{chunk_count, decrypt_chunk, StreamWriter, CHUNK_LEN, PREFIX_LEN};
use crate::crypto::{padme, random_array, sha384};
use crate::error::{CoreError, CoreResult};

pub const MAGIC: &[u8; 12] = b"ATROPOS-VLT\x01";
const AREA: u64 = 64 * 1024;
const MAX_STREAM_HEADER: u32 = 4096;
const MAX_INDEX: u64 = 256 * 1024 * 1024;
/// Data inside the file's own stream; any other blob id is pending.
pub const STREAM: Id = [0u8; 16];
const STREAM_START: u64 = MAGIC.len() as u64 + 2 * AREA;

#[derive(Serialize, Deserialize)]
struct StreamHeader {
    #[serde(with = "super::b64")]
    prefix: Vec<u8>,
}

/// True if `path` is a single-file vault.
pub fn is_single(path: &Path) -> bool {
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut m = [0u8; 12];
    f.read_exact(&mut m).is_ok() && &m == MAGIC
}

fn not_a_vault() -> CoreError {
    CoreError::InvalidInput {
        message: "this file is not an Atropos vault, or it is damaged".into(),
    }
}

/// Both key areas that parse, newest first.
pub fn read_headers(path: &Path) -> CoreResult<Vec<(u64, HeaderFile)>> {
    let mut f = fs::File::open(path).map_err(|_| CoreError::InvalidInput {
        message: format!("no Atropos vault at {}", path.display()),
    })?;
    let mut m = [0u8; 12];
    f.read_exact(&mut m).map_err(|_| not_a_vault())?;
    if &m != MAGIC {
        return Err(not_a_vault());
    }
    let mut out = Vec::new();
    for i in 0..2u64 {
        let mut area = vec![0u8; AREA as usize];
        f.seek(SeekFrom::Start(MAGIC.len() as u64 + i * AREA))?;
        if f.read_exact(&mut area).is_err() {
            continue;
        }
        let len = u32::from_be_bytes(area[..4].try_into().unwrap_or_default()) as usize;
        if len == 0 || len > AREA as usize - 12 {
            continue;
        }
        let seq = u64::from_be_bytes(area[4..12].try_into().unwrap_or_default());
        if let Ok(h) = HeaderFile::parse(&area[12..12 + len]) {
            out.push((seq, h));
        }
    }
    if out.is_empty() {
        return Err(not_a_vault());
    }
    out.sort_by_key(|h| std::cmp::Reverse(h.0));
    Ok(out)
}

fn area_bytes(header: &HeaderFile, seq: u64) -> CoreResult<Vec<u8>> {
    let json = serde_json::to_vec(header)?;
    if json.len() > AREA as usize - 12 {
        return Err(CoreError::InvalidInput {
            message: "this vault has too many unlock methods; remove one first".into(),
        });
    }
    let mut area = Vec::with_capacity(AREA as usize);
    area.extend_from_slice(&(json.len() as u32).to_be_bytes());
    area.extend_from_slice(&seq.to_be_bytes());
    area.extend_from_slice(&json);
    area.resize(AREA as usize, 0);
    Ok(area)
}

fn stream_key(secrets: &VaultSecrets, vault_id: &[u8]) -> CoreResult<Zeroizing<[u8; 32]>> {
    secrets.subkey(b"single-file-stream", vault_id)
}

fn stream_aad(header: &HeaderFile, stream_header: &[u8]) -> Vec<u8> {
    let pk = &header.body.public_key;
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&header.body.vault_id);
    aad.extend_from_slice(&sha384(&[&pk.mlkem, &pk.p384]));
    aad.extend_from_slice(&(stream_header.len() as u32).to_be_bytes());
    aad.extend_from_slice(stream_header);
    aad
}

/// Where a file's bytes wait until the next write of the vault file.
pub enum Pending {
    Memory(Zeroizing<Vec<u8>>),
    /// An original on disk (an import), read when the vault is written.
    Disk(PathBuf),
}

/// An open single-file vault's data: the encrypted stream plus changes
/// not yet written.
pub struct SingleFile {
    path: PathBuf,
    file: fs::File,
    key: AeadKey,
    prefix: [u8; PREFIX_LEN],
    aad: Vec<u8>,
    /// Where the encrypted stream starts in the file.
    start: u64,
    sealed: u64,
    seq: u64,
    pending: HashMap<Id, Pending>,
}

/// Writes a complete vault file at `out`: `header` in both key areas and
/// the files of `entries`, each read through `read` (shared data is written
/// once). Returns the entries as they are in the new file.
#[allow(clippy::too_many_arguments)]
pub fn write_file(
    out: &Path,
    header: &HeaderFile,
    seq: u64,
    secrets: &VaultSecrets,
    entries: &BTreeMap<Id, Entry>,
    read: &mut dyn FnMut(&Entry, &mut dyn Write) -> CoreResult<()>,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<BTreeMap<Id, Entry>> {
    let key = stream_key(secrets, &header.body.vault_id)?;
    let prefix = random_array::<PREFIX_LEN>()?;
    let sh = serde_json::to_vec(&StreamHeader {
        prefix: prefix.to_vec(),
    })?;
    let aad = stream_aad(header, &sh);
    let files: Vec<&Entry> = entries
        .values()
        .filter(|e| e.kind == Kind::File && e.data.is_some())
        .collect();
    let mut p = Progress {
        files_total: files.len() as u64,
        bytes_total: files.iter().map(|e| e.size).sum(),
        ..Default::default()
    };
    progress(&p);
    let mut fresh = entries.clone();
    let result = (|| -> CoreResult<()> {
        let mut file = BufWriter::new(fs::File::create(out)?);
        file.write_all(MAGIC)?;
        let area = area_bytes(header, seq)?;
        file.write_all(&area)?;
        file.write_all(&area)?;
        file.write_all(&(sh.len() as u32).to_be_bytes())?;
        file.write_all(&sh)?;
        let mut w = StreamWriter::new(file, &key, prefix, &aad)?;
        // Data shared by copies is written once.
        let mut placed: HashMap<(Id, u64, u64), (u64, String)> = HashMap::new();
        for e in files {
            let d = e.data.clone().ok_or(CoreError::Crypto)?;
            p.current = e.name.clone();
            let (offset, digest) = match placed.get(&(d.blob, d.offset, d.len)) {
                Some(found) => found.clone(),
                None => {
                    let offset = w.written;
                    let mut h = Hashing {
                        inner: &mut w,
                        hasher: Sha256::new(),
                        len: 0,
                    };
                    read(e, &mut h)?;
                    let (len, digest) = (h.len, hex::encode(h.hasher.finalize()));
                    if len != d.len {
                        return Err(CoreError::InvalidInput {
                            message: format!(
                                "{} changed while it was being added; try again",
                                e.name
                            ),
                        });
                    }
                    placed.insert((d.blob, d.offset, d.len), (offset, digest.clone()));
                    (offset, digest)
                }
            };
            let f = fresh.get_mut(&e.id).ok_or(CoreError::Crypto)?;
            if f.sha256.as_deref().is_some_and(|s| s != digest) {
                return Err(CoreError::InvalidInput {
                    message: format!("{} does not read back correctly", e.name),
                });
            }
            f.sha256 = Some(digest);
            f.data = Some(DataRef {
                blob: STREAM,
                offset,
                len: d.len,
            });
            p.files_done += 1;
            p.bytes_done += d.len;
            progress(&p);
        }
        let index_at = w.written;
        let index = Zeroizing::new(serde_json::to_vec(&fresh.values().collect::<Vec<_>>())?);
        w.write_all(&index)?;
        let used = w.written + 16;
        let pad = padme(used.max(4096)) - used;
        std::io::copy(&mut std::io::repeat(0).take(pad), &mut w)?;
        w.write_all(&index_at.to_be_bytes())?;
        w.write_all(&(index.len() as u64).to_be_bytes())?;
        let file = w.finish()?;
        file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(out);
        return Err(e);
    }
    Ok(fresh)
}

struct Hashing<'a, W: Write> {
    inner: &'a mut W,
    hasher: Sha256,
    len: u64,
}

impl<W: Write> Write for Hashing<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl SingleFile {
    /// Opens the stream of the vault file at `path`, whose key area `seq`
    /// held `header`. Returns it with the file list.
    pub fn open(
        path: &Path,
        header: &HeaderFile,
        seq: u64,
        secrets: &VaultSecrets,
    ) -> CoreResult<(Self, BTreeMap<Id, Entry>)> {
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(STREAM_START))?;
        let mut len = [0u8; 4];
        file.read_exact(&mut len).map_err(|_| not_a_vault())?;
        let len = u32::from_be_bytes(len);
        if len > MAX_STREAM_HEADER {
            return Err(not_a_vault());
        }
        let mut sh = vec![0u8; len as usize];
        file.read_exact(&mut sh).map_err(|_| not_a_vault())?;
        let parsed: StreamHeader = serde_json::from_slice(&sh).map_err(|_| not_a_vault())?;
        let prefix: [u8; PREFIX_LEN] = parsed
            .prefix
            .as_slice()
            .try_into()
            .map_err(|_| not_a_vault())?;
        let start = STREAM_START + 4 + len as u64;
        let total = file.metadata()?.len();
        let sealed = total.checked_sub(start).ok_or(CoreError::DecryptFailed)?;
        let key = stream_key(secrets, &header.body.vault_id)?;
        let mut s = SingleFile {
            path: path.to_path_buf(),
            file,
            key: AeadKey::new(&key)?,
            prefix,
            aad: stream_aad(header, &sh),
            start,
            sealed,
            seq,
            pending: HashMap::new(),
        };
        let chunks = chunk_count(sealed);
        let plain_len = sealed
            .checked_sub(chunks * TAG_LEN as u64)
            .filter(|l| *l >= 16)
            .ok_or(CoreError::DecryptFailed)?;
        let mut trailer = Zeroizing::new(Vec::new());
        // The last chunk is authenticated here, and with it the length.
        s.read_range(plain_len - 16, 16, &mut *trailer)?;
        let at = u64::from_be_bytes(
            trailer[..8]
                .try_into()
                .map_err(|_| CoreError::DecryptFailed)?,
        );
        let len = u64::from_be_bytes(
            trailer[8..]
                .try_into()
                .map_err(|_| CoreError::DecryptFailed)?,
        );
        if len > MAX_INDEX || at.checked_add(len).is_none_or(|end| end > plain_len - 16) {
            return Err(CoreError::DecryptFailed);
        }
        let mut index = Zeroizing::new(Vec::new());
        s.read_range(at, len, &mut *index)?;
        let list: Vec<Entry> =
            serde_json::from_slice(&index).map_err(|_| CoreError::DecryptFailed)?;
        let mut entries = BTreeMap::new();
        for e in list {
            if let Some(d) = &e.data {
                if d.blob != STREAM || d.offset.checked_add(d.len).is_none_or(|end| end > at) {
                    return Err(CoreError::DecryptFailed);
                }
            }
            entries.insert(e.id, e);
        }
        Ok((s, entries))
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Writes `header` into both key areas, newer sequence first.
    pub fn write_header(&mut self, header: &HeaderFile) -> CoreResult<()> {
        let seq = self.seq + 1;
        let area = area_bytes(header, seq)?;
        let mut f = fs::OpenOptions::new().write(true).open(&self.path)?;
        for i in 0..2u64 {
            f.seek(SeekFrom::Start(MAGIC.len() as u64 + i * AREA))?;
            f.write_all(&area)?;
            f.sync_data()?;
        }
        self.seq = seq;
        Ok(())
    }

    /// Keeps `p` until the next write. Returns the blob id that refers to it.
    pub fn add_pending(&mut self, p: Pending) -> CoreResult<Id> {
        let id = loop {
            let id = random_array::<16>()?;
            if id != STREAM && !self.pending.contains_key(&id) {
                break id;
            }
        };
        self.pending.insert(id, p);
        Ok(id)
    }

    pub fn drop_pending(&mut self, blob: &Id) {
        self.pending.remove(blob);
    }

    /// Writes the bytes `d` refers to into `out`.
    pub fn read(&mut self, d: &DataRef, out: &mut dyn Write) -> CoreResult<()> {
        if d.blob == STREAM {
            return self.read_range(d.offset, d.len, out);
        }
        match self.pending.get(&d.blob) {
            Some(Pending::Memory(b)) => {
                let end = d
                    .offset
                    .checked_add(d.len)
                    .ok_or(CoreError::DecryptFailed)?;
                let slice = b
                    .get(d.offset as usize..end as usize)
                    .ok_or(CoreError::DecryptFailed)?;
                out.write_all(slice)?;
                Ok(())
            }
            Some(Pending::Disk(path)) => {
                let mut f = fs::File::open(path)?;
                f.seek(SeekFrom::Start(d.offset))?;
                let n = std::io::copy(&mut f.take(d.len + 1), out)?;
                if n != d.len {
                    return Err(CoreError::InvalidInput {
                        message: format!(
                            "{} changed while it was being added; try again",
                            path.display()
                        ),
                    });
                }
                Ok(())
            }
            None => Err(CoreError::DecryptFailed),
        }
    }

    fn chunk(&mut self, index: u64) -> CoreResult<Zeroizing<Vec<u8>>> {
        let sealed_chunk = (CHUNK_LEN + TAG_LEN) as u64;
        let chunks = chunk_count(self.sealed);
        if index >= chunks {
            return Err(CoreError::DecryptFailed);
        }
        let at = index * sealed_chunk;
        let size = sealed_chunk.min(self.sealed - at) as usize;
        let mut buf = vec![0u8; size];
        self.file.seek(SeekFrom::Start(self.start + at))?;
        self.file.read_exact(&mut buf)?;
        decrypt_chunk(
            &self.key,
            &self.prefix,
            &self.aad,
            index,
            index + 1 == chunks,
            &buf,
        )
    }

    fn read_range(&mut self, from: u64, len: u64, out: &mut dyn Write) -> CoreResult<()> {
        let chunk_len = CHUNK_LEN as u64;
        let mut pos = from;
        let end = from.checked_add(len).ok_or(CoreError::DecryptFailed)?;
        while pos < end {
            let plain = self.chunk(pos / chunk_len)?;
            let inner = (pos % chunk_len) as usize;
            let take = ((end - pos) as usize).min(plain.len().saturating_sub(inner));
            if take == 0 {
                return Err(CoreError::DecryptFailed);
            }
            out.write_all(&plain[inner..inner + take])?;
            pos += take as u64;
        }
        Ok(())
    }

    /// Writes the vault file again with `entries` (pending data included)
    /// and swaps it in. Returns the entries as stored now.
    pub fn rewrite(
        &mut self,
        header: &HeaderFile,
        secrets: &VaultSecrets,
        entries: &BTreeMap<Id, Entry>,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<BTreeMap<Id, Entry>> {
        let tmp = self.path.with_extension("atropos-part");
        let seq = self.seq + 1;
        let fresh = {
            let mut read = |e: &Entry, w: &mut dyn Write| -> CoreResult<()> {
                let d = e.data.clone().ok_or(CoreError::Crypto)?;
                self.read(&d, w)
            };
            write_file(&tmp, header, seq, secrets, entries, &mut read, progress)?
        };
        // Let go of the old file first (Windows will not replace an open one).
        self.file = fs::File::open(&tmp)?;
        if let Err(e) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            self.file = fs::File::open(&self.path)?;
            return Err(e.into());
        }
        let (again, _) = SingleFile::open(&self.path, header, seq, secrets)?;
        *self = again;
        Ok(fresh)
    }
}
