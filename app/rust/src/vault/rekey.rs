//! "Re-key vault": moves everything into a vault with all-new keys.
//!
//! A new key pair, root secret, vault id and recovery phrase are made, every
//! file is decrypted and encrypted again under them, and the new vault takes
//! the old one's place. Plaintext only ever passes through memory. Copies
//! of the old vault made earlier (backups, cloud history) still open with
//! the old password or phrase; this copy no longer does with the old phrase.
//! Fingerprint and card slots are not carried over.
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use zeroize::Zeroizing;

use super::header::{SlotKind, Unlock};
use super::manifest::{Entry, Id, Kind};
use super::{Pack, Progress, Vault};
use crate::crypto::password::Argon2Cost;
use crate::crypto::random_array;
use crate::error::{CoreError, CoreResult};

const PIECE: usize = 256 * 1024;

/// The writing half of an in-memory pipe. Every piece is wiped once read.
struct PipeWriter(mpsc::SyncSender<Zeroizing<Vec<u8>>>);

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = buf.len().min(PIECE);
        self.0
            .send(Zeroizing::new(buf[..n].to_vec()))
            .map_err(|_| io::Error::other("the copy stopped"))?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: mpsc::Receiver<Zeroizing<Vec<u8>>>,
    cur: Zeroizing<Vec<u8>>,
    pos: usize,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.cur.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.cur = next;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Entries parents-first, so each folder exists before what is inside it.
fn ordered(v: &Vault) -> Vec<Entry> {
    let m = v.manifest();
    let mut out = Vec::new();
    let mut level: Vec<Option<Id>> = vec![None];
    while !level.is_empty() {
        let mut next = Vec::new();
        for parent in level {
            for e in m.children(parent) {
                if e.kind == Kind::Dir {
                    next.push(Some(e.id));
                }
                out.push(e.clone());
            }
        }
        level = next;
    }
    out
}

fn sibling(path: &Path, tag: &str) -> CoreResult<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::InvalidInput {
            message: "the vault folder has no name".into(),
        })?
        .to_string_lossy()
        .to_string();
    let rand = hex::encode(random_array::<4>()?);
    Ok(path.with_file_name(format!(".{name}.{tag}-{rand}")))
}

impl Vault {
    /// Re-encrypts the whole vault under new keys (see the module docs).
    /// Needs the current password, which stays the same. Returns the new
    /// vault, already in the old one's folder, and its new recovery phrase.
    pub fn rekey(
        &mut self,
        password: &[u8],
        cost: Argon2Cost,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Zeroizing<String>> {
        let (_, slot) = self
            .header
            .body
            .unlock(&Unlock::Password(password))
            .map_err(|_| wrong_password())?;
        if self.header.body.slots[slot].kind != SlotKind::Password {
            return Err(wrong_password());
        }
        let path = self.path.clone();
        let tmp = sibling(&path, "rekey")?;
        let result = (|| -> CoreResult<Zeroizing<String>> {
            let (mut new, phrase) = Vault::create(&tmp, password, cost)?;
            self.copy_into(&mut new, progress)?;
            // Read everything back through the new keys before switching.
            let report = new.check(&mut |_| {})?;
            if !report.damaged.is_empty() || !report.missing.is_empty() {
                return Err(CoreError::InvalidInput {
                    message: "the re-encrypted copy did not verify; nothing was changed".into(),
                });
            }
            Ok(phrase)
        })();
        let phrase = match result {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e);
            }
        };
        // Swap: old aside, new in place, then remove the old one.
        let old = sibling(&path, "old")?;
        std::fs::rename(&path, &old)?;
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::rename(&old, &path);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e.into());
        }
        let _ = std::fs::remove_dir_all(&old);
        *self = Vault::unlock(&path, &Unlock::Password(password))?;
        Ok(phrase)
    }

    fn copy_into(
        &mut self,
        new: &mut Vault,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<()> {
        let entries = ordered(self);
        let mut p = Progress {
            files_total: entries.iter().filter(|e| e.kind == Kind::File).count() as u64,
            bytes_total: entries
                .iter()
                .filter(|e| e.kind == Kind::File)
                .map(|e| e.size)
                .sum(),
            ..Progress::default()
        };
        progress(&p);
        let mut pack = Pack::new();
        for e in entries {
            let mut copy = e.clone();
            copy.data = None;
            if e.kind != Kind::File {
                new.put_entry(copy)?;
                continue;
            }
            p.current = e.name.clone();
            let (tx, rx) = mpsc::sync_channel::<Zeroizing<Vec<u8>>>(4);
            let size = e.size;
            let id = e.id;
            let (read, written) = std::thread::scope(|s| {
                let reader = s.spawn(|| {
                    let mut w = PipeWriter(tx);
                    self.read_file(id, &mut w)
                });
                let mut r = PipeReader {
                    rx,
                    cur: Zeroizing::new(Vec::new()),
                    pos: 0,
                };
                let written = new.add_file(copy, &mut r, &mut pack);
                // Let the reader finish even if writing stopped early.
                drop(r);
                (reader.join(), written)
            });
            read.map_err(|_| CoreError::Crypto)??;
            written?;
            p.files_done += 1;
            p.bytes_done += size;
            progress(&p);
        }
        new.flush_pack(&mut pack)?;
        Ok(())
    }
}

/// If a re-key was interrupted between moving the old vault aside and
/// moving the new one in, puts the old one back. Called when `path` has no
/// vault.
pub fn recover_interrupted(path: &Path) {
    if path.exists() {
        return;
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!(".{}.old-", name.to_string_lossy());
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(&prefix)
            && e.path().join(super::HEADER_FILE).exists()
            && std::fs::rename(e.path(), path).is_ok()
        {
            return;
        }
    }
}

fn wrong_password() -> CoreError {
    CoreError::InvalidInput {
        message: "that is not this vault's password".into(),
    }
}
