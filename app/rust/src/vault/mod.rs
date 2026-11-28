//! Atropos vaults: an ordinary folder of encrypted, randomly named blobs plus
//! an encrypted file list and one clear-text header.
//!
//! ```text
//! <vault>/
//!   vault.hdr      public key, key slots, header MAC (see header.rs)
//!   manifest/      encrypted journal of the file tree (see manifest.rs)
//!   objects/xx/    encrypted file contents (see blob.rs)
//! ```
pub(crate) mod b64;
pub mod blob;
pub mod check;
pub mod header;
pub mod manifest;
pub mod package;
pub mod recovery;
pub mod rekey;
pub mod single;

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use self::blob::BlobReader;
use self::header::{
    CardSlotJson, HeaderBody, HeaderFile, SlotKind, Unlock, VaultSecrets, DEVICE_TTL_SECS,
};
use self::manifest::{DataRef, Entry, Id, Kind, Manifest, Record};
use crate::crypto::password::Argon2Cost;
use crate::crypto::random_array;
use crate::error::{CoreError, CoreResult};

pub const HEADER_FILE: &str = "vault.hdr";
const MANIFEST_DIR: &str = "manifest";
const OBJECTS_DIR: &str = "objects";
/// Files up to this size are packed together into shared blobs.
pub const SMALL_FILE: u64 = 64 * 1024;
/// A pack is written once it holds this much.
const PACK_TARGET: usize = 4 * 1024 * 1024;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mtime(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

pub fn is_vault(path: &Path) -> bool {
    path.join(HEADER_FILE).is_file() || single::is_single(path)
}

/// Reads the header of a locked vault (for showing its name and slots).
pub fn read_header(path: &Path) -> CoreResult<HeaderFile> {
    if path.is_file() {
        return single::read_headers(path).map(|mut h| h.remove(0).1);
    }
    rekey::recover_interrupted(path);
    let bytes = fs::read(path.join(HEADER_FILE)).map_err(|_| CoreError::InvalidInput {
        message: format!("no Atropos vault at {}", path.display()),
    })?;
    HeaderFile::parse(&bytes)
}

/// Progress of an import or export.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

/// Summary after an import or export finishes.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub skipped: Vec<String>,
    pub originals_deleted: bool,
}

/// An unlocked vault. Dropping it wipes every key it holds.
pub struct Vault {
    path: PathBuf,
    header: HeaderFile,
    secrets: VaultSecrets,
    manifest: Manifest,
    store: Store,
    /// What the last check found, for [Vault::repair].
    last_check: Option<check::CheckReport>,
}

/// Where a vault keeps file contents.
pub(crate) enum Store {
    /// A folder vault ("Vault LFS"): one encrypted blob file per file or pack.
    Folder(BlobReader),
    /// Everything in one file.
    Single(single::SingleFile),
    /// Keys only, no files (see [Vault::key_clone]).
    KeyOnly,
}

struct PendingFile {
    entry: Entry,
    offset: u64,
    len: u64,
}

/// Collects small files into one pack blob.
pub(crate) struct Pack {
    buf: Zeroizing<Vec<u8>>,
    files: Vec<PendingFile>,
}

impl Pack {
    pub(crate) fn new() -> Self {
        Self {
            buf: Zeroizing::new(Vec::new()),
            files: Vec::new(),
        }
    }
}

impl Vault {
    /// Creates a new vault in the empty or new folder `path`. Returns the
    /// unlocked vault and the recovery phrase to show once.
    pub fn create(
        path: &Path,
        password: &[u8],
        cost: Argon2Cost,
    ) -> CoreResult<(Vault, Zeroizing<String>)> {
        if path.exists() && fs::read_dir(path)?.next().is_some() {
            return Err(CoreError::InvalidInput {
                message: "pick an empty folder for the new vault".into(),
            });
        }
        crate::api::secure_input::check_strength(password)?;
        fs::create_dir_all(path)?;
        let secrets = VaultSecrets::generate()?;
        let mut body = HeaderBody::new(&secrets, now())?;
        body.add_slot(&secrets, &Unlock::Password(password), cost)?;
        let recovery_key = Zeroizing::new(random_array::<32>()?);
        body.add_slot(&secrets, &Unlock::Recovery(&recovery_key), cost)?;
        let header = body.seal(&secrets)?;
        fs::create_dir_all(path.join(OBJECTS_DIR))?;
        let mkey = secrets.subkey(b"manifest", &header.body.vault_id)?;
        let manifest = Manifest::load(&path.join(MANIFEST_DIR), &mkey, &header.body.vault_id)?;
        manifest::write_atomic(&path.join(HEADER_FILE), &[&header.to_bytes()?])?;
        let phrase = recovery::to_phrase(&recovery_key)?;
        let blobs = BlobReader::new(&path.join(OBJECTS_DIR), &header.body.vault_id);
        Ok((
            Vault {
                path: path.to_path_buf(),
                header,
                secrets,
                manifest,
                store: Store::Folder(blobs),
                last_check: None,
            },
            phrase,
        ))
    }

    /// Creates a new single-file vault at `path` (a file that must not
    /// exist yet). Returns it unlocked with the recovery phrase to show once.
    pub fn create_single(
        path: &Path,
        password: &[u8],
        cost: Argon2Cost,
    ) -> CoreResult<(Vault, Zeroizing<String>)> {
        if path.exists() {
            return Err(CoreError::InvalidInput {
                message: "a file with that name is already there".into(),
            });
        }
        let (header, secrets, phrase) = new_keys(password, cost)?;
        let vault = Self::write_single(
            path,
            header,
            secrets,
            &Default::default(),
            &mut |_, _| Ok(()),
            &mut |_| {},
        )?;
        Ok((vault, phrase))
    }

    /// Turns an older passphrase-locked `.atropos` file into a single-file
    /// vault in place. The passphrase becomes the vault's password, and a
    /// recovery phrase is made (returned, to show once).
    pub fn upgrade_package(
        path: &Path,
        passphrase: &[u8],
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<(Vault, Zeroizing<String>)> {
        let mut reader =
            package::PackageReader::open(path, package::PackageKey::Passphrase(passphrase))?;
        let entries = reader.vault_entries()?;
        let (header, secrets, phrase) = new_keys_unchecked(passphrase, Argon2Cost::DEFAULT)?;
        let tmp = path.with_extension("atropos-part");
        single::write_file(
            &tmp,
            &header,
            1,
            &secrets,
            &entries,
            &mut |e, w| {
                let d = e.data.as_ref().ok_or(CoreError::Crypto)?;
                reader.read_file(d.offset as usize, w)
            },
            progress,
        )?;
        // Let go of the old file first (Windows will not replace an open one).
        drop(reader);
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        let vault = Vault::unlock(path, &Unlock::Password(passphrase))?;
        Ok((vault, phrase))
    }

    /// Writes a copy of this vault as the other kind at `out` (a folder
    /// vault becomes a single file, a single file becomes a folder vault),
    /// with the same password, recovery phrase and other unlock methods.
    /// This vault stays as it is.
    pub fn convert(&mut self, out: &Path, progress: &mut dyn FnMut(&Progress)) -> CoreResult<()> {
        self.persist(&mut |_| {})?;
        if out.exists() && !(out.is_dir() && fs::read_dir(out)?.next().is_none()) {
            return Err(CoreError::InvalidInput {
                message: "pick a new name for the copy; something is already there".into(),
            });
        }
        let secrets = self.clone_secrets()?;
        if self.is_single() {
            fs::create_dir_all(out)?;
            let result = (|| -> CoreResult<()> {
                fs::create_dir_all(out.join(OBJECTS_DIR))?;
                let vault_id = self.header.body.vault_id.clone();
                let mkey = secrets.subkey(b"manifest", &vault_id)?;
                let manifest = Manifest::load(&out.join(MANIFEST_DIR), &mkey, &vault_id)?;
                manifest::write_atomic(&out.join(HEADER_FILE), &[&self.header.to_bytes()?])?;
                let mut new = Vault {
                    path: out.to_path_buf(),
                    header: self.header.clone(),
                    secrets,
                    manifest,
                    store: Store::Folder(BlobReader::new(&out.join(OBJECTS_DIR), &vault_id)),
                    last_check: None,
                };
                self.copy_into(&mut new, progress)
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(out);
            }
            result
        } else {
            let entries = self.manifest.entries.clone();
            let header = self.header.clone();
            Self::write_single(
                out,
                header,
                secrets,
                &entries,
                &mut |e, w| self.read_file(e.id, &mut &mut *w).map(|_| ()),
                progress,
            )
            .map(|_| ())
        }
    }

    fn clone_secrets(&self) -> CoreResult<VaultSecrets> {
        let (m, p) = self.secrets.keypair.secret_parts();
        Ok(VaultSecrets {
            keypair: crate::crypto::hybrid::HybridSecretKey::from_parts(
                &m,
                &p,
                self.public_key()?,
            )?,
            root: self.secrets.root.clone(),
        })
    }

    /// Writes a single-file vault at `path` (through a part file) with
    /// `header` and `entries`, and opens it.
    fn write_single(
        path: &Path,
        header: HeaderFile,
        secrets: VaultSecrets,
        entries: &std::collections::BTreeMap<Id, Entry>,
        read: &mut dyn FnMut(&Entry, &mut dyn Write) -> CoreResult<()>,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Vault> {
        let tmp = path.with_extension("atropos-part");
        single::write_file(&tmp, &header, 1, &secrets, entries, read, progress)?;
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        let (file, entries) = single::SingleFile::open(path, &header, 1, &secrets)?;
        Ok(Vault {
            path: path.to_path_buf(),
            header,
            secrets,
            manifest: Manifest::in_memory(entries)?,
            store: Store::Single(file),
            last_check: None,
        })
    }

    /// True for a single-file vault, false for a folder vault (LFS).
    pub fn is_single(&self) -> bool {
        matches!(self.store, Store::Single(_))
    }

    /// Writes a single-file vault's changes into its file. Does nothing for
    /// a folder vault, which writes each change as it happens.
    pub fn persist(&mut self, progress: &mut dyn FnMut(&Progress)) -> CoreResult<()> {
        let Store::Single(file) = &mut self.store else {
            return Ok(());
        };
        if !self.manifest.changed && !file.has_pending() {
            return Ok(());
        }
        let fresh = file.rewrite(
            &self.header,
            &self.secrets,
            &self.manifest.entries,
            progress,
        )?;
        self.manifest.entries = fresh;
        self.manifest.changed = false;
        Ok(())
    }

    /// Forgets changes not yet written (after a failed [Vault::persist]).
    pub fn discard_changes(&mut self) -> CoreResult<()> {
        if !self.is_single() {
            return Ok(());
        }
        let mut candidates = single::read_headers(&self.path)?;
        let (seq, header) = candidates.remove(0);
        let (file, entries) = single::SingleFile::open(&self.path, &header, seq, &self.secrets)?;
        self.header = header;
        self.manifest = Manifest::in_memory(entries)?;
        self.store = Store::Single(file);
        Ok(())
    }

    /// Writes the bytes `d` points to (in a blob, or in the single file).
    pub(crate) fn read_data(&mut self, d: &DataRef, out: &mut dyn Write) -> CoreResult<()> {
        match &mut self.store {
            Store::Folder(blobs) => blobs.read_range(
                &self.secrets.keypair,
                &d.blob,
                d.offset,
                d.len,
                &mut &mut *out,
            ),
            Store::Single(file) => file.read(d, out),
            Store::KeyOnly => Err(CoreError::Crypto),
        }
    }

    /// Deletes blob `b` once no entry uses it.
    fn free_blob(&mut self, b: &Id) {
        if self.manifest.blob_in_use(b) {
            return;
        }
        match &mut self.store {
            Store::Folder(blobs) => blobs.remove(b),
            Store::Single(file) => file.drop_pending(b),
            Store::KeyOnly => {}
        }
    }

    /// The card slots of a locked vault (read from its header).
    pub fn card_slots(path: &Path) -> CoreResult<Vec<CardSlotJson>> {
        let h = read_header(path)?;
        Ok(h.body.slots.iter().filter_map(|s| s.card.clone()).collect())
    }

    pub fn unlock(path: &Path, how: &Unlock) -> CoreResult<Vault> {
        if path.is_file() {
            return Self::unlock_single(path, how);
        }
        let header = read_header(path)?;
        let (secrets, slot) = header.body.unlock(how)?;
        header.verify(&secrets)?;
        check_device_expiry(&header, slot, how)?;
        let vault_id = header.body.vault_id.clone();
        let mkey = secrets.subkey(b"manifest", &vault_id)?;
        let manifest = Manifest::load(&path.join(MANIFEST_DIR), &mkey, &vault_id)?;
        let mut vault = Vault {
            path: path.to_path_buf(),
            header,
            secrets,
            manifest,
            store: Store::Folder(BlobReader::new(&path.join(OBJECTS_DIR), &vault_id)),
            last_check: None,
        };
        // Re-seal the slot that opened so its stored bytes change each unlock.
        // A read-only vault (e.g. on a CD) still opens.
        let mut body = vault.header.body.clone();
        if body.reseal(slot, &vault.secrets, how).is_ok() {
            let _ = vault.save_header(body);
        }
        Ok(vault)
    }

    fn unlock_single(path: &Path, how: &Unlock) -> CoreResult<Vault> {
        let candidates = single::read_headers(path)?;
        let mut last = CoreError::DecryptFailed;
        // The newest key area first; the other one only if it was cut off.
        for (seq, header) in candidates {
            let opened = header
                .body
                .unlock(how)
                .and_then(|(secrets, slot)| header.verify(&secrets).map(|_| (secrets, slot)));
            let (secrets, slot) = match opened {
                Ok(o) => o,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            check_device_expiry(&header, slot, how)?;
            let (file, entries) = single::SingleFile::open(path, &header, seq, &secrets)?;
            let mut vault = Vault {
                path: path.to_path_buf(),
                header,
                secrets,
                manifest: Manifest::in_memory(entries)?,
                store: Store::Single(file),
                last_check: None,
            };
            let mut body = vault.header.body.clone();
            if body.reseal(slot, &vault.secrets, how).is_ok() {
                let _ = vault.save_header(body);
            }
            return Ok(vault);
        }
        Err(last)
    }

    fn save_header(&mut self, body: HeaderBody) -> CoreResult<()> {
        let sealed = body.seal(&self.secrets)?;
        match &mut self.store {
            Store::Single(file) => file.write_header(&sealed)?,
            _ => manifest::write_atomic(&self.path.join(HEADER_FILE), &[&sealed.to_bytes()?])?,
        }
        self.header = sealed;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn vault_id(&self) -> &[u8] {
        &self.header.body.vault_id
    }

    /// A second handle with the same keys but no file list, used as the key
    /// when a package sealed to this vault is imported into it.
    pub(crate) fn key_clone(&self) -> CoreResult<Vault> {
        let secrets = self.clone_secrets()?;
        Ok(Vault {
            path: self.path.clone(),
            header: self.header.clone(),
            secrets,
            manifest: Manifest::in_memory(Default::default())?,
            store: Store::KeyOnly,
            last_check: None,
        })
    }

    pub fn public_key(&self) -> CoreResult<crate::crypto::hybrid::HybridPublicKey> {
        self.header.body.public_key()
    }

    pub(crate) fn decapsulate(
        &self,
        ct: &crate::crypto::hybrid::HybridCiphertext,
    ) -> CoreResult<Zeroizing<[u8; 32]>> {
        self.secrets.keypair.decapsulate(ct)
    }

    pub fn created(&self) -> i64 {
        self.header.body.created
    }

    /// Replaces the password slot. The old password stops working for this
    /// copy of the vault (old copies elsewhere still open with it).
    pub fn change_password(&mut self, new_password: &[u8], cost: Argon2Cost) -> CoreResult<()> {
        crate::api::secure_input::check_strength(new_password)?;
        let mut body = self.header.body.clone();
        body.remove_slots(SlotKind::Password);
        body.add_slot(&self.secrets, &Unlock::Password(new_password), cost)?;
        self.save_header(body)
    }

    /// The Argon2id cost protecting the password slot.
    pub fn password_cost(&self) -> Option<Argon2Cost> {
        self.header
            .body
            .slots
            .iter()
            .find(|s| s.kind == SlotKind::Password)
            .and_then(|s| s.argon2)
            .map(Argon2Cost::from)
    }

    /// Re-wraps the password slot with a different Argon2id cost. Needs the
    /// current password, which is checked against the slot first.
    pub fn set_password_cost(&mut self, password: &[u8], cost: Argon2Cost) -> CoreResult<()> {
        cost.validate()?;
        let (_, slot) = self
            .header
            .body
            .unlock(&Unlock::Password(password))
            .map_err(|_| CoreError::InvalidInput {
                message: "that is not this vault's password".into(),
            })?;
        if self.header.body.slots[slot].kind != SlotKind::Password {
            return Err(CoreError::InvalidInput {
                message: "that is not this vault's password".into(),
            });
        }
        let mut body = self.header.body.clone();
        body.remove_slots(SlotKind::Password);
        body.add_slot(&self.secrets, &Unlock::Password(password), cost)?;
        self.save_header(body)
    }

    /// Adds (or replaces) this device's biometric slot and returns the new
    /// secret for the platform keystore to protect. It opens the vault for
    /// [DEVICE_TTL_SECS] after each strong unlock on this device.
    pub fn enroll_device(&mut self, device: &[u8], label: &str) -> CoreResult<Zeroizing<[u8; 32]>> {
        if device.is_empty() || device.len() > 64 {
            return Err(CoreError::InvalidInput {
                message: "bad device id".into(),
            });
        }
        let secret = Zeroizing::new(random_array::<32>()?);
        let mut body = self.header.body.clone();
        body.remove_device(device);
        body.add_slot(
            &self.secrets,
            &Unlock::Device {
                device,
                secret: &secret,
            },
            Argon2Cost::DEFAULT,
        )?;
        let t = now();
        let slot = body.device_slot_mut(device).ok_or(CoreError::Crypto)?;
        slot.label = Some(label.chars().take(64).collect());
        slot.expires = Some(t + DEVICE_TTL_SECS);
        slot.renewed = Some(t);
        self.save_header(body)?;
        Ok(secret)
    }

    pub fn remove_device(&mut self, device: &[u8]) -> CoreResult<()> {
        if self.header.body.device_slot(device).is_none() {
            return Ok(());
        }
        let mut body = self.header.body.clone();
        body.remove_device(device);
        self.save_header(body)
    }

    /// After a password or recovery unlock: this device's biometric slot works
    /// for another [DEVICE_TTL_SECS].
    pub fn renew_device(&mut self, device: &[u8]) -> CoreResult<()> {
        if self.header.body.device_slot(device).is_none() {
            return Ok(());
        }
        let mut body = self.header.body.clone();
        let t = now();
        if let Some(slot) = body.device_slot_mut(device) {
            slot.expires = Some(t + DEVICE_TTL_SECS);
            slot.renewed = Some(t);
        }
        self.save_header(body)
    }

    /// Adds a card slot. `shared` is the ECDH secret for `card` (computed
    /// locally from the ephemeral key, and checked against the card).
    pub fn add_card(&mut self, card: CardSlotJson, shared: &[u8], label: &str) -> CoreResult<()> {
        let mut body = self.header.body.clone();
        // Adding the same card again replaces its slot.
        body.slots.retain(|s| {
            !(s.kind == SlotKind::Card && s.card.as_ref().map(|c| &c.public) == Some(&card.public))
        });
        body.add_slot(
            &self.secrets,
            &Unlock::Card {
                card: &card,
                shared,
            },
            Argon2Cost::DEFAULT,
        )?;
        let slot = body.slots.last_mut().ok_or(CoreError::Crypto)?;
        slot.label = Some(label.chars().take(64).collect());
        slot.renewed = Some(now());
        self.save_header(body)
    }

    /// Card slots: (slot id, label, added at, card).
    pub fn cards(&self) -> Vec<(Vec<u8>, String, i64, CardSlotJson)> {
        self.header
            .body
            .slots
            .iter()
            .filter_map(|s| {
                let c = s.card.clone()?;
                Some((
                    s.id.clone(),
                    s.label.clone().unwrap_or_default(),
                    s.renewed.unwrap_or(0),
                    c,
                ))
            })
            .collect()
    }

    pub fn remove_card(&mut self, slot_id: &[u8]) -> CoreResult<()> {
        let mut body = self.header.body.clone();
        let before = body.slots.len();
        body.slots
            .retain(|s| !(s.kind == SlotKind::Card && s.id == slot_id));
        if body.slots.len() == before {
            return Ok(());
        }
        self.save_header(body)
    }

    /// When this device's biometric slot stops working, if it has one.
    pub fn device_expires(&self, device: &[u8]) -> Option<i64> {
        self.header.body.device_slot(device).and_then(|s| s.expires)
    }

    /// Replaces the recovery key and returns the new phrase.
    pub fn new_recovery_key(&mut self) -> CoreResult<Zeroizing<String>> {
        let key = Zeroizing::new(random_array::<32>()?);
        let mut body = self.header.body.clone();
        body.remove_slots(SlotKind::Recovery);
        body.add_slot(&self.secrets, &Unlock::Recovery(&key), Argon2Cost::DEFAULT)?;
        self.save_header(body)?;
        recovery::to_phrase(&key)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn check_parent(&self, parent: Option<Id>) -> CoreResult<()> {
        match parent {
            None => Ok(()),
            Some(p) => match self.manifest.get(&p) {
                Some(e) if e.kind == Kind::Dir => Ok(()),
                _ => Err(CoreError::InvalidInput {
                    message: "that folder no longer exists".into(),
                }),
            },
        }
    }

    pub fn create_folder(&mut self, parent: Option<Id>, name: &str) -> CoreResult<Id> {
        let name = clean_name(name)?;
        self.check_parent(parent)?;
        let id = random_array::<16>()?;
        let entry = Entry {
            id,
            parent,
            name: self.manifest.free_name(parent, &name, None),
            kind: Kind::Dir,
            size: 0,
            modified: now(),
            executable: false,
            link_target: None,
            data: None,
            sha256: None,
            open_as: None,
        };
        self.manifest.commit(vec![Record::Put { entry }])?;
        Ok(id)
    }

    pub fn rename(&mut self, id: Id, name: &str) -> CoreResult<()> {
        let name = clean_name(name)?;
        let mut entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        entry.name = self.manifest.free_name(entry.parent, &name, Some(id));
        self.manifest.commit(vec![Record::Put { entry }])
    }

    /// Moves `id` into `parent`, refusing to move a folder into itself.
    pub fn move_to(&mut self, id: Id, parent: Option<Id>) -> CoreResult<()> {
        self.check_parent(parent)?;
        if let Some(p) = parent {
            if self.manifest.subtree(id).contains(&p) {
                return Err(CoreError::InvalidInput {
                    message: "a folder cannot be moved into itself".into(),
                });
            }
        }
        let mut entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        entry.parent = parent;
        entry.name = self.manifest.free_name(parent, &entry.name, Some(id));
        self.manifest.commit(vec![Record::Put { entry }])
    }

    /// Deletes entries (folders with everything inside) and any blobs nothing
    /// else uses.
    pub fn delete(&mut self, ids: &[Id]) -> CoreResult<()> {
        let mut doomed = Vec::new();
        for id in ids {
            if self.manifest.get(id).is_some() {
                doomed.extend(self.manifest.subtree(*id));
            }
        }
        doomed.sort();
        doomed.dedup();
        let blobs: Vec<Id> = doomed
            .iter()
            .filter_map(|id| {
                self.manifest
                    .get(id)
                    .and_then(|e| e.data.as_ref().map(|d| d.blob))
            })
            .collect();
        self.manifest
            .commit(doomed.iter().map(|id| Record::Remove { id: *id }).collect())?;
        for b in blobs {
            self.free_blob(&b);
        }
        Ok(())
    }

    /// Writes the plaintext of file `id` to `out`, checking its SHA-256.
    pub fn read_file(&mut self, id: Id, out: &mut impl Write) -> CoreResult<u64> {
        let entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        if entry.kind != Kind::File {
            return Err(CoreError::InvalidInput {
                message: "that is not a file".into(),
            });
        }
        let Some(data) = entry.data else {
            return Ok(0);
        };
        let mut hashing = HashWriter::new(out);
        self.read_data(&data, &mut hashing)?;
        if let Some(expected) = &entry.sha256 {
            if &hex::encode(hashing.hasher.finalize()) != expected {
                return Err(CoreError::DecryptFailed);
            }
        }
        Ok(data.len)
    }

    /// Encrypts files and folders from disk into `parent`. With
    /// `delete_originals`, each original is deleted only after everything
    /// was imported and verified by decrypting it again.
    pub fn import(
        &mut self,
        sources: &[PathBuf],
        parent: Option<Id>,
        delete_originals: bool,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Outcome> {
        self.check_parent(parent)?;
        let mut p = Progress::default();
        for s in sources {
            count(s, &mut p)?;
        }
        progress(&p);
        let mut outcome = Outcome::default();
        let mut pack = Pack::new();
        let mut imported: Vec<PathBuf> = Vec::new();
        for s in sources {
            self.import_one(
                s,
                parent,
                &mut pack,
                &mut p,
                &mut outcome,
                &mut imported,
                progress,
            )?;
        }
        self.flush_pack(&mut pack)?;
        if self.is_single() {
            if let Err(e) = self.persist(progress) {
                let _ = self.discard_changes();
                return Err(e);
            }
            if delete_originals {
                // Read every new file back from the vault file first.
                let ids: Vec<Id> = self.manifest.entries.keys().copied().collect();
                for id in ids {
                    if self.manifest.get(&id).is_some_and(|e| e.kind == Kind::File) {
                        self.read_file(id, &mut std::io::sink())?;
                    }
                }
            }
        }
        if delete_originals {
            for path in imported.iter().rev() {
                let meta = fs::symlink_metadata(path)?;
                if meta.is_dir() {
                    let _ = fs::remove_dir(path);
                } else {
                    fs::remove_file(path)?;
                }
            }
            outcome.originals_deleted = true;
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn import_one(
        &mut self,
        src: &Path,
        parent: Option<Id>,
        pack: &mut Pack,
        p: &mut Progress,
        outcome: &mut Outcome,
        imported: &mut Vec<PathBuf>,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<()> {
        let meta = fs::symlink_metadata(src)?;
        let raw_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".into());
        let name = self.unique_pending_name(parent, &raw_name, pack);
        let id = random_array::<16>()?;
        let mut entry = Entry {
            id,
            parent,
            name,
            kind: Kind::File,
            size: 0,
            modified: mtime(&meta),
            executable: is_executable(&meta),
            link_target: None,
            data: None,
            sha256: None,
            open_as: None,
        };
        p.current = raw_name;
        if meta.file_type().is_symlink() {
            entry.kind = Kind::Symlink;
            entry.link_target = Some(fs::read_link(src)?.to_string_lossy().to_string());
            entry.executable = false;
            self.manifest.commit(vec![Record::Put { entry }])?;
            imported.push(src.to_path_buf());
            outcome.files += 1;
            p.files_done += 1;
            progress(p);
        } else if meta.is_dir() {
            entry.kind = Kind::Dir;
            entry.executable = false;
            self.manifest.commit(vec![Record::Put { entry }])?;
            imported.push(src.to_path_buf());
            outcome.folders += 1;
            let mut children: Vec<PathBuf> = fs::read_dir(src)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<_, _>>()?;
            children.sort();
            for c in children {
                self.import_one(&c, Some(id), pack, p, outcome, imported, progress)?;
            }
        } else if meta.is_file() {
            let len = meta.len();
            entry.size = len;
            if let (Store::Single(file), true) = (&mut self.store, len > 0) {
                // Read when the vault file is written, not held in memory.
                let blob = file.add_pending(single::Pending::Disk(src.to_path_buf()))?;
                entry.data = Some(DataRef {
                    blob,
                    offset: 0,
                    len,
                });
                self.manifest.commit(vec![Record::Put { entry }])?;
            } else {
                let mut file = fs::File::open(src)?;
                self.add_file(entry, &mut file, pack).map_err(|e| match e {
                    CoreError::Io { .. } => changed(src),
                    e => e,
                })?;
            }
            imported.push(src.to_path_buf());
            outcome.files += 1;
            outcome.bytes += len;
            p.files_done += 1;
            p.bytes_done += len;
            progress(p);
        } else {
            outcome.skipped.push(src.display().to_string());
        }
        Ok(())
    }

    /// Encrypts `entry.size` bytes from `reader` as file `entry`: small files
    /// go into `pack`, larger ones get their own blob, verified before the
    /// entry is recorded.
    pub(crate) fn add_file(
        &mut self,
        mut entry: Entry,
        reader: &mut dyn Read,
        pack: &mut Pack,
    ) -> CoreResult<()> {
        let len = entry.size;
        entry.kind = Kind::File;
        if len == 0 {
            entry.sha256 = Some(hex::encode(Sha256::digest([])));
            return self.manifest.commit(vec![Record::Put { entry }]);
        }
        if len <= SMALL_FILE {
            let mut data = Zeroizing::new(vec![0u8; len as usize]);
            reader.read_exact(&mut data)?;
            let mut extra = [0u8; 1];
            if reader.read(&mut extra)? != 0 {
                return Err(CoreError::Io {
                    message: "the file grew while it was being imported".into(),
                });
            }
            entry.sha256 = Some(hex::encode(Sha256::digest(&data[..])));
            let offset = pack.buf.len() as u64;
            pack.buf.extend_from_slice(&data);
            pack.files.push(PendingFile { entry, offset, len });
            if pack.buf.len() >= PACK_TARGET {
                self.flush_pack(pack)?;
            }
            return Ok(());
        }
        if let Store::Single(file) = &mut self.store {
            let mut data = Zeroizing::new(Vec::with_capacity(len as usize));
            reader.take(len + 1).read_to_end(&mut data)?;
            if data.len() as u64 != len {
                return Err(CoreError::Io {
                    message: "the file changed while it was being added".into(),
                });
            }
            entry.sha256 = Some(hex::encode(Sha256::digest(&data[..])));
            let blob = file.add_pending(single::Pending::Memory(data))?;
            entry.data = Some(DataRef {
                blob,
                offset: 0,
                len,
            });
            return self.manifest.commit(vec![Record::Put { entry }]);
        }
        let mut hashing = HashReader::new(reader);
        let public = self.header.body.public_key()?;
        let blob = blob::write_blob(
            &self.path.join(OBJECTS_DIR),
            self.vault_id(),
            &public,
            &mut hashing,
            len,
        )?;
        let digest = hex::encode(hashing.hasher.finalize());
        self.verify_blob(&blob, &[(0, len, digest.as_str())])?;
        entry.sha256 = Some(digest);
        entry.data = Some(DataRef {
            blob,
            offset: 0,
            len,
        });
        self.manifest.commit(vec![Record::Put { entry }])
    }

    /// Replaces the contents of file `id` (an in-app edit). The new data is
    /// encrypted into a fresh blob; the old one is deleted once unused.
    pub fn replace_contents(&mut self, id: Id, data: &[u8]) -> CoreResult<()> {
        let mut entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        if entry.kind != Kind::File {
            return Err(CoreError::InvalidInput {
                message: "that is not a file".into(),
            });
        }
        let old = entry.data.take().map(|d| d.blob);
        entry.size = data.len() as u64;
        entry.modified = now();
        entry.sha256 = None;
        let mut pack = Pack::new();
        self.add_file(entry, &mut &data[..], &mut pack)?;
        self.flush_pack(&mut pack)?;
        if let Some(b) = old {
            self.free_blob(&b);
        }
        Ok(())
    }

    /// Adds a new file holding `data` next to `like` (same folder, a free
    /// name based on `name`). Returns its id.
    pub fn add_copy(&mut self, like: Id, name: &str, data: &[u8]) -> CoreResult<Id> {
        let src = self.manifest.get(&like).cloned().ok_or_else(gone)?;
        let name = clean_name(name)?;
        let id = random_array::<16>()?;
        let entry = Entry {
            id,
            parent: src.parent,
            name: self.manifest.free_name(src.parent, &name, None),
            kind: Kind::File,
            size: data.len() as u64,
            modified: now(),
            executable: false,
            link_target: None,
            data: None,
            sha256: None,
            open_as: src.open_as.clone(),
        };
        let mut pack = Pack::new();
        self.add_file(entry, &mut &data[..], &mut pack)?;
        self.flush_pack(&mut pack)?;
        Ok(id)
    }

    /// Creates a new file called `name` (a free name) in `parent`, holding
    /// `data`. Returns its id.
    pub fn create_file(&mut self, parent: Option<Id>, name: &str, data: &[u8]) -> CoreResult<Id> {
        let name = clean_name(name)?;
        self.check_parent(parent)?;
        let id = random_array::<16>()?;
        let entry = Entry {
            id,
            parent,
            name: self.manifest.free_name(parent, &name, None),
            kind: Kind::File,
            size: data.len() as u64,
            modified: now(),
            executable: false,
            link_target: None,
            data: None,
            sha256: None,
            open_as: None,
        };
        let mut pack = Pack::new();
        self.add_file(entry, &mut &data[..], &mut pack)?;
        self.flush_pack(&mut pack)?;
        Ok(id)
    }

    /// Copies `id` (a folder with everything in it) into `parent`. Copies
    /// share the encrypted data with the original, so this is instant; the
    /// data is only deleted once nothing points to it. Returns the new id.
    pub fn copy_to(&mut self, id: Id, parent: Option<Id>) -> CoreResult<Id> {
        self.check_parent(parent)?;
        if let Some(p) = parent {
            if self.manifest.subtree(id).contains(&p) {
                return Err(CoreError::InvalidInput {
                    message: "a folder cannot be copied into itself".into(),
                });
            }
        }
        let top = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        let mut records = Vec::new();
        let mut map: HashMap<Id, Id> = HashMap::new();
        let new_top = random_array::<16>()?;
        map.insert(id, new_top);
        let mut entry = top.clone();
        entry.id = new_top;
        entry.parent = parent;
        entry.name = self.manifest.free_name(parent, &top.name, None);
        records.push(Record::Put { entry });
        // subtree lists parents before their children.
        for old in self.manifest.subtree(id).into_iter().skip(1) {
            let mut e = self.manifest.get(&old).cloned().ok_or_else(gone)?;
            let new = random_array::<16>()?;
            map.insert(old, new);
            e.id = new;
            e.parent = e.parent.and_then(|p| map.get(&p).copied());
            records.push(Record::Put { entry: e });
        }
        self.manifest.commit(records)?;
        Ok(new_top)
    }

    /// Remembers how file `id` should open ("text", "image", ...), or
    /// forgets it with None.
    pub fn set_open_as(&mut self, id: Id, open_as: Option<String>) -> CoreResult<()> {
        let mut entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        if entry.open_as == open_as {
            return Ok(());
        }
        entry.open_as = open_as;
        self.manifest.commit(vec![Record::Put { entry }])
    }

    pub(crate) fn put_entry(&mut self, entry: Entry) -> CoreResult<()> {
        self.manifest.commit(vec![Record::Put { entry }])
    }

    /// A free name that also avoids names still waiting in the pack.
    pub(crate) fn unique_pending_name(
        &self,
        parent: Option<Id>,
        wanted: &str,
        pack: &Pack,
    ) -> String {
        let mut name = self.manifest.free_name(parent, wanted, None);
        let mut n = 2;
        while pack
            .files
            .iter()
            .any(|f| f.entry.parent == parent && f.entry.name.eq_ignore_ascii_case(&name))
        {
            let base = self.manifest.free_name(parent, wanted, None);
            name = format!("{base} ({n})");
            n += 1;
        }
        name
    }

    pub(crate) fn flush_pack(&mut self, pack: &mut Pack) -> CoreResult<()> {
        if pack.files.is_empty() {
            return Ok(());
        }
        if let Store::Single(file) = &mut self.store {
            let data = std::mem::replace(&mut pack.buf, Zeroizing::new(Vec::new()));
            let blob = file.add_pending(single::Pending::Memory(data))?;
            let records = pack
                .files
                .drain(..)
                .map(|f| {
                    let mut entry = f.entry;
                    entry.data = Some(DataRef {
                        blob,
                        offset: f.offset,
                        len: f.len,
                    });
                    Record::Put { entry }
                })
                .collect();
            return self.manifest.commit(records);
        }
        let public = self.header.body.public_key()?;
        let blob = blob::write_blob(
            &self.path.join(OBJECTS_DIR),
            self.vault_id(),
            &public,
            &pack.buf[..],
            pack.buf.len() as u64,
        )?;
        let checks: Vec<(u64, u64, String)> = pack
            .files
            .iter()
            .map(|f| (f.offset, f.len, f.entry.sha256.clone().unwrap_or_default()))
            .collect();
        let refs: Vec<(u64, u64, &str)> = checks
            .iter()
            .map(|(o, l, h)| (*o, *l, h.as_str()))
            .collect();
        self.verify_blob(&blob, &refs)?;
        let records = pack
            .files
            .drain(..)
            .map(|f| {
                let mut entry = f.entry;
                entry.data = Some(DataRef {
                    blob,
                    offset: f.offset,
                    len: f.len,
                });
                Record::Put { entry }
            })
            .collect();
        self.manifest.commit(records)?;
        pack.buf.clear();
        Ok(())
    }

    /// Decrypts the new blob again and compares each file's SHA-256, so an
    /// import only counts once the data is known to read back.
    fn verify_blob(&mut self, blob: &Id, files: &[(u64, u64, &str)]) -> CoreResult<()> {
        for (offset, len, digest) in files {
            let mut h = HashWriter::new(std::io::sink());
            let d = DataRef {
                blob: *blob,
                offset: *offset,
                len: *len,
            };
            self.read_data(&d, &mut h)?;
            if hex::encode(h.hasher.finalize()) != *digest {
                if let Store::Folder(blobs) = &mut self.store {
                    blobs.remove(blob);
                }
                return Err(CoreError::InvalidInput {
                    message: "verification failed: the encrypted copy does not read back; nothing was imported".into(),
                });
            }
        }
        Ok(())
    }

    /// Writes entries out as normal files into the folder `dest`.
    pub fn export(
        &mut self,
        ids: &[Id],
        dest: &Path,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Outcome> {
        fs::create_dir_all(dest)?;
        let mut p = Progress::default();
        for id in ids {
            for sub in self.manifest.subtree(*id) {
                if let Some(e) = self.manifest.get(&sub) {
                    if e.kind == Kind::File {
                        p.files_total += 1;
                        p.bytes_total += e.size;
                    }
                }
            }
        }
        progress(&p);
        let mut outcome = Outcome::default();
        for id in ids {
            self.export_one(*id, dest, &mut p, &mut outcome, progress)?;
        }
        Ok(outcome)
    }

    fn export_one(
        &mut self,
        id: Id,
        dest: &Path,
        p: &mut Progress,
        outcome: &mut Outcome,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<()> {
        let entry = self.manifest.get(&id).cloned().ok_or_else(gone)?;
        let target = free_disk_name(dest, &entry.name);
        p.current = entry.name.clone();
        match entry.kind {
            Kind::Dir => {
                fs::create_dir_all(&target)?;
                outcome.folders += 1;
                let children: Vec<Id> = self
                    .manifest
                    .children(Some(id))
                    .iter()
                    .map(|e| e.id)
                    .collect();
                for c in children {
                    self.export_one(c, &target, p, outcome, progress)?;
                }
            }
            Kind::Symlink => {
                #[cfg(unix)]
                {
                    let t = entry.link_target.clone().unwrap_or_default();
                    std::os::unix::fs::symlink(t, &target)?;
                }
                #[cfg(not(unix))]
                outcome
                    .skipped
                    .push(format!("{} (symbolic link)", entry.name));
            }
            Kind::File => {
                let tmp = target.with_extension("atropos-part");
                let result = (|| -> CoreResult<()> {
                    let mut out = std::io::BufWriter::new(fs::File::create(&tmp)?);
                    self.read_file(id, &mut out)?;
                    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
                    Ok(())
                })();
                if let Err(e) = result {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
                fs::rename(&tmp, &target)?;
                set_mtime(&target, entry.modified);
                #[cfg(unix)]
                if entry.executable {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perm = fs::metadata(&target)?.permissions();
                    perm.set_mode(perm.mode() | 0o755);
                    fs::set_permissions(&target, perm)?;
                }
                outcome.files += 1;
                outcome.bytes += entry.size;
                p.files_done += 1;
                p.bytes_done += entry.size;
                progress(p);
            }
        }
        Ok(())
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        if let Store::Folder(blobs) = &mut self.store {
            blobs.forget_keys();
        }
    }
}

/// New vault keys with a password and a recovery slot, and the recovery
/// phrase to show once.
pub(crate) fn new_keys(
    password: &[u8],
    cost: Argon2Cost,
) -> CoreResult<(HeaderFile, VaultSecrets, Zeroizing<String>)> {
    crate::api::secure_input::check_strength(password)?;
    new_keys_unchecked(password, cost)
}

/// [new_keys] without the strength check (for a passphrase already in use).
fn new_keys_unchecked(
    password: &[u8],
    cost: Argon2Cost,
) -> CoreResult<(HeaderFile, VaultSecrets, Zeroizing<String>)> {
    let secrets = VaultSecrets::generate()?;
    let mut body = HeaderBody::new(&secrets, now())?;
    body.add_slot(&secrets, &Unlock::Password(password), cost)?;
    let recovery_key = Zeroizing::new(random_array::<32>()?);
    body.add_slot(&secrets, &Unlock::Recovery(&recovery_key), cost)?;
    let header = body.seal(&secrets)?;
    Ok((header, secrets, recovery::to_phrase(&recovery_key)?))
}

/// A fingerprint (device) slot opens only within its time limit. The expiry
/// is trusted only once the header MAC checked out.
fn check_device_expiry(header: &HeaderFile, slot: usize, how: &Unlock) -> CoreResult<()> {
    if how.is_strong() {
        return Ok(());
    }
    let s = &header.body.slots[slot];
    let t = now();
    let expired = s.expires.is_none_or(|e| t > e);
    let clock_back = s.renewed.is_some_and(|r| t + 300 < r);
    if expired || clock_back {
        return Err(CoreError::InvalidInput {
            message: "fingerprint unlock has expired. Unlock once with the \
                      password or recovery phrase and it works again for 24 hours"
                .into(),
        });
    }
    Ok(())
}

fn count(src: &Path, p: &mut Progress) -> CoreResult<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        for e in fs::read_dir(src)? {
            count(&e?.path(), p)?;
        }
    } else {
        p.files_total += 1;
        if meta.is_file() {
            p.bytes_total += meta.len();
        }
    }
    Ok(())
}

/// Trims a name and refuses ones that cannot be a single path component.
pub fn clean_name(name: &str) -> CoreResult<String> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\', '\0']) {
        return Err(CoreError::InvalidInput {
            message: "names cannot be empty or contain / or \\".into(),
        });
    }
    Ok(n.to_string())
}

/// A file name in `dir` that does not exist yet.
pub(crate) fn free_disk_name(dir: &Path, wanted: &str) -> PathBuf {
    let safe: String = wanted
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let first = dir.join(&safe);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match safe.rfind('.') {
        Some(i) if i > 0 => (&safe[..i], &safe[i..]),
        _ => (safe.as_str(), ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .unwrap_or(first)
}

fn gone() -> CoreError {
    CoreError::InvalidInput {
        message: "that item no longer exists".into(),
    }
}

fn changed(p: &Path) -> CoreError {
    CoreError::InvalidInput {
        message: format!(
            "{} changed while it was being imported; try again",
            p.display()
        ),
    }
}

pub(crate) fn file_mtime(meta: &fs::Metadata) -> i64 {
    mtime(meta)
}

pub(crate) fn file_executable(meta: &fs::Metadata) -> bool {
    is_executable(meta)
}

pub(crate) fn unique_disk_name(dir: &Path, wanted: &str) -> PathBuf {
    free_disk_name(dir, wanted)
}

struct HashReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R> HashReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }
}

impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

struct HashWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W> HashWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests;

/// Best effort: gives a restored file its original modification time.
pub(crate) fn set_mtime(path: &Path, unix_secs: i64) {
    use std::time::{Duration, UNIX_EPOCH};
    let t = if unix_secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(unix_secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(unix_secs.unsigned_abs())
    };
    if let Ok(f) = fs::File::options().write(true).open(path) {
        let _ = f.set_modified(t);
    }
}
