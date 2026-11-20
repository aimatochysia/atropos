//! The vault API the Flutter UI calls. Dart only ever holds an opaque
//! `VaultSession`; keys stay inside it in Rust and are wiped on `lock()`.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use flutter_rust_bridge::frb;
use zeroize::{Zeroize, Zeroizing};

use super::secure_input::SecretInput;
use crate::crypto::password::Argon2Cost;
use crate::error::{CoreError, CoreResult};
use crate::frb_generated::StreamSink;
use crate::util::path_to_string;
use crate::vault::header::Unlock;
use crate::vault::manifest::{Id, Kind};
use crate::vault::package::{self, Destination, PackageKey, PackageLock, SealTo};
use crate::vault::{self, recovery, Outcome, Progress, Vault};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    Link,
}

pub struct EntryInfo {
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
    pub executable: bool,
    /// Number of direct children, for folders.
    pub children: u32,
}

pub struct VaultInfo {
    pub path: String,
    pub name: String,
    pub created: i64,
    /// Hex; names this vault's biometric key in the platform keystore.
    pub vault_id: String,
}

#[derive(Clone)]
pub struct ProgressInfo {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

pub struct OutcomeInfo {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub skipped: Vec<String>,
    pub originals_deleted: bool,
}

/// Progress events of a long task; the last one is `Done`.
pub enum TaskEvent {
    Progress(ProgressInfo),
    Done(OutcomeInfo),
}

pub struct PackageInfo {
    /// True when a passphrase opens it; false when it is locked to a vault.
    pub needs_passphrase: bool,
    /// For vault-locked packages: the id of the vault that can open it.
    pub vault_id: String,
}

pub(crate) fn progress_info(p: &Progress) -> ProgressInfo {
    ProgressInfo {
        files_done: p.files_done,
        files_total: p.files_total,
        bytes_done: p.bytes_done,
        bytes_total: p.bytes_total,
        current: p.current.clone(),
    }
}

fn outcome_info(o: Outcome) -> OutcomeInfo {
    OutcomeInfo {
        files: o.files,
        folders: o.folders,
        bytes: o.bytes,
        skipped: o.skipped,
        originals_deleted: o.originals_deleted,
    }
}

/// Reports progress at most every ~100 ms, plus the first and last update.
fn reporter(sink: &StreamSink<TaskEvent>) -> impl FnMut(&Progress) + '_ {
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    move |p: &Progress| {
        let done = p.files_done == p.files_total;
        if done || last.elapsed().as_millis() >= 100 {
            last = std::time::Instant::now();
            let _ = sink.add(TaskEvent::Progress(progress_info(p)));
        }
    }
}

pub(crate) fn parse_id(id: &str) -> CoreResult<Id> {
    hex::decode(id)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(CoreError::InvalidInput {
            message: "that item no longer exists".into(),
        })
}

fn parse_ids(ids: &[String]) -> CoreResult<Vec<Id>> {
    ids.iter().map(|i| parse_id(i)).collect()
}

fn parse_parent(parent: Option<String>) -> CoreResult<Option<Id>> {
    parent.map(|p| parse_id(&p)).transpose()
}

fn vault_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_to_string(path))
}

/// Whether `path` is an Atropos vault folder.
#[frb(sync)]
pub fn is_vault(path: String) -> bool {
    vault::is_vault(Path::new(&path))
}

/// Reads a locked vault's public details.
pub fn inspect_vault(path: String) -> CoreResult<VaultInfo> {
    let p = PathBuf::from(&path);
    let h = vault::read_header(&p)?;
    Ok(VaultInfo {
        name: vault_name(&p),
        path,
        created: h.body.created,
        vault_id: hex::encode(&h.body.vault_id),
    })
}

/// An unlocked vault. Clones share the same keys; `lock()` on any of them
/// locks all.
#[frb(opaque)]
#[derive(Clone)]
pub struct VaultSession {
    inner: Arc<Mutex<Option<Vault>>>,
    path: String,
    vault_id: String,
    /// Open viewers; their plaintext is wiped when the vault locks.
    pub(crate) viewers: Arc<Mutex<Vec<Weak<crate::view::Slot>>>>,
}

#[derive(Clone)]
pub struct RekeyDone {
    /// The re-keyed vault, already unlocked. The old session is locked.
    pub session: VaultSession,
    /// Shown once; the old phrase no longer opens this copy.
    pub recovery_phrase: String,
}

/// A step of a re-key (opaque because it carries a session).
pub enum RekeyEvent {
    Progress(ProgressInfo),
    Done(RekeyDone),
}

impl RekeyEvent {
    #[frb(sync)]
    pub fn progress(&self) -> Option<ProgressInfo> {
        match self {
            RekeyEvent::Progress(p) => Some(p.clone()),
            RekeyEvent::Done(_) => None,
        }
    }

    #[frb(sync)]
    pub fn done(&self) -> Option<RekeyDone> {
        match self {
            RekeyEvent::Done(d) => Some(d.clone()),
            RekeyEvent::Progress(_) => None,
        }
    }
}

pub struct CreatedVault {
    pub session: VaultSession,
    /// Shown once so the person can write it down; never stored.
    pub recovery_phrase: String,
}

pub(crate) fn new_session(v: Vault) -> VaultSession {
    session(v)
}

fn session(v: Vault) -> VaultSession {
    VaultSession {
        path: path_to_string(v.path()),
        vault_id: hex::encode(v.vault_id()),
        inner: Arc::new(Mutex::new(Some(v))),
        viewers: Arc::default(),
    }
}

/// Creates the vault folder `parent_dir/name` with `password`.
pub fn create_vault(
    parent_dir: String,
    name: String,
    password: &SecretInput,
) -> CoreResult<CreatedVault> {
    let name = vault::clean_name(&name)?;
    let path = Path::new(&parent_dir).join(name);
    let (v, phrase) = Vault::create(&path, &password.bytes(), Argon2Cost::DEFAULT)?;
    Ok(CreatedVault {
        session: session(v),
        recovery_phrase: phrase.to_string(),
    })
}

/// Renews this device's biometric unlock for 24 hours after a strong unlock.
pub(crate) fn strong_session(v: Vault, device_id: Option<String>) -> VaultSession {
    strong(v, device_id)
}

pub(crate) fn session_of<T>(
    s: &VaultSession,
    f: impl FnOnce(&mut Vault) -> CoreResult<T>,
) -> CoreResult<T> {
    s.with(f)
}

fn strong(mut v: Vault, device_id: Option<String>) -> VaultSession {
    if let Some(d) = device_id.and_then(|d| hex::decode(d).ok()) {
        let _ = v.renew_device(&d);
    }
    session(v)
}

/// `device_id` (from `device_id()`) renews biometric unlock on this device.
pub fn unlock_with_password(
    path: String,
    password: &SecretInput,
    device_id: Option<String>,
) -> CoreResult<VaultSession> {
    let pw = password.bytes();
    Vault::unlock(Path::new(&path), &Unlock::Password(&pw))
        .map(|v| strong(v, device_id))
        .map_err(wrong_secret)
}

pub fn unlock_with_recovery_phrase(
    path: String,
    phrase: &SecretInput,
    device_id: Option<String>,
) -> CoreResult<VaultSession> {
    let key = recovery::from_phrase(&phrase.text())?;
    Vault::unlock(Path::new(&path), &Unlock::Recovery(&key))
        .map(|v| strong(v, device_id))
        .map_err(wrong_secret)
}

/// Unlocks with the secret the platform keystore released after a
/// biometric check. `secret` is wiped here.
pub fn unlock_with_device(
    path: String,
    device_id: String,
    mut secret: Vec<u8>,
) -> CoreResult<VaultSession> {
    let device = hex::decode(&device_id).map_err(|_| CoreError::InvalidInput {
        message: "bad device id".into(),
    })?;
    let key: Zeroizing<[u8; 32]> = Zeroizing::new(
        secret
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::DecryptFailed)?,
    );
    secret.zeroize();
    Vault::unlock(
        Path::new(&path),
        &Unlock::Device {
            device: &device,
            secret: &key,
        },
    )
    .map(session)
    .map_err(|e| match e {
        CoreError::DecryptFailed => CoreError::InvalidInput {
            message: "fingerprint unlock is no longer set up for this vault on this \
                      device; unlock with the password"
                .into(),
        },
        e => e,
    })
}

fn parse_device(id: &str) -> CoreResult<Vec<u8>> {
    hex::decode(id).map_err(|_| CoreError::InvalidInput {
        message: "bad device id".into(),
    })
}

pub(crate) fn wrong_secret(e: CoreError) -> CoreError {
    match e {
        CoreError::DecryptFailed => CoreError::InvalidInput {
            message: "wrong password or recovery phrase".into(),
        },
        e => e,
    }
}

/// Checks a recovery phrase's words and checksum without unlocking.
#[frb(sync)]
pub fn recovery_phrase_is_valid(phrase: &SecretInput) -> bool {
    recovery::from_phrase(&phrase.text()).is_ok()
}

fn locked() -> CoreError {
    CoreError::InvalidInput {
        message: "the vault is locked".into(),
    }
}

impl VaultSession {
    /// Runs `f` on the vault and, when it succeeds, takes the vault out of
    /// this session (which then reads as locked) and returns it.
    pub(crate) fn take_for_rekey<T>(
        &self,
        f: impl FnOnce(&mut Vault) -> CoreResult<T>,
    ) -> CoreResult<(Vault, T)> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let v = g.as_mut().ok_or_else(locked)?;
        let out = f(v)?;
        let vault = g.take().ok_or_else(locked)?;
        drop(g);
        let viewers = std::mem::take(&mut *self.viewers.lock().unwrap_or_else(|e| e.into_inner()));
        for v in viewers.iter().filter_map(Weak::upgrade) {
            *v.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok((vault, out))
    }

    /// Like `with`, but gives up instead of waiting while a task runs.
    pub(crate) fn try_with<T>(&self, f: impl FnOnce(&Vault) -> T) -> Option<T> {
        let g = self.inner.try_lock().ok()?;
        g.as_ref().map(f)
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Vault) -> CoreResult<T>) -> CoreResult<T> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let v = g.as_mut().ok_or_else(locked)?;
        f(v)
    }

    #[frb(sync, getter)]
    pub fn path(&self) -> String {
        self.path.clone()
    }

    #[frb(sync, getter)]
    pub fn name(&self) -> String {
        vault_name(Path::new(&self.path))
    }

    /// When the vault was created (Unix seconds); 0 once locked.
    #[frb(sync)]
    pub fn created(&self) -> i64 {
        // Never blocks the UI thread while a long task holds the vault.
        match self.inner.try_lock() {
            Ok(g) => g.as_ref().map(|v| v.created()).unwrap_or(0),
            Err(_) => 0,
        }
    }

    #[frb(sync)]
    pub fn is_locked(&self) -> bool {
        match self.inner.try_lock() {
            Ok(g) => g.is_none(),
            Err(std::sync::TryLockError::WouldBlock) => false,
            Err(std::sync::TryLockError::Poisoned(_)) => true,
        }
    }

    /// Wipes every key of this vault from memory. Runs off the UI thread:
    /// a task in progress finishes its current step first.
    pub fn lock(&self) {
        let viewers = std::mem::take(&mut *self.viewers.lock().unwrap_or_else(|e| e.into_inner()));
        for v in viewers.iter().filter_map(Weak::upgrade) {
            *v.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *g = None;
    }

    /// Items directly inside `parent` (None = the top level), folders first.
    pub fn list(&self, parent: Option<String>) -> CoreResult<Vec<EntryInfo>> {
        let parent = parse_parent(parent)?;
        self.with(|v| {
            let m = v.manifest();
            Ok(m.children(parent)
                .into_iter()
                .map(|e| EntryInfo {
                    id: hex::encode(e.id),
                    name: e.name.clone(),
                    kind: match e.kind {
                        Kind::File => EntryKind::File,
                        Kind::Dir => EntryKind::Folder,
                        Kind::Symlink => EntryKind::Link,
                    },
                    size: e.size,
                    modified: e.modified,
                    executable: e.executable,
                    children: if e.kind == Kind::Dir {
                        m.children(Some(e.id)).len() as u32
                    } else {
                        0
                    },
                })
                .collect())
        })
    }

    /// Total files and bytes in the vault.
    pub fn totals(&self) -> CoreResult<OutcomeInfo> {
        self.with(|v| {
            let mut o = OutcomeInfo {
                files: 0,
                folders: 0,
                bytes: 0,
                skipped: vec![],
                originals_deleted: false,
            };
            for e in v.manifest().entries.values() {
                match e.kind {
                    Kind::Dir => o.folders += 1,
                    _ => {
                        o.files += 1;
                        o.bytes += e.size;
                    }
                }
            }
            Ok(o)
        })
    }

    pub fn create_folder(&self, parent: Option<String>, name: String) -> CoreResult<String> {
        let parent = parse_parent(parent)?;
        self.with(|v| v.create_folder(parent, &name).map(hex::encode))
    }

    pub fn rename(&self, id: String, name: String) -> CoreResult<()> {
        let id = parse_id(&id)?;
        self.with(|v| v.rename(id, &name))
    }

    pub fn move_to(&self, ids: Vec<String>, parent: Option<String>) -> CoreResult<()> {
        let ids = parse_ids(&ids)?;
        let parent = parse_parent(parent)?;
        self.with(|v| ids.iter().try_for_each(|id| v.move_to(*id, parent)))
    }

    pub fn delete(&self, ids: Vec<String>) -> CoreResult<()> {
        let ids = parse_ids(&ids)?;
        self.with(|v| v.delete(&ids))
    }

    /// Encrypts files/folders from disk into the vault.
    pub fn import(
        &self,
        sources: Vec<String>,
        parent: Option<String>,
        delete_originals: bool,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let parent = parse_parent(parent)?;
        let sources: Vec<PathBuf> = sources.into_iter().map(PathBuf::from).collect();
        let o =
            self.with(|v| v.import(&sources, parent, delete_originals, &mut reporter(&sink)))?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    /// Writes items out as normal, unencrypted files into `dest_dir`.
    pub fn export(
        &self,
        ids: Vec<String>,
        dest_dir: String,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let ids = parse_ids(&ids)?;
        let o = self.with(|v| v.export(&ids, Path::new(&dest_dir), &mut reporter(&sink)))?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    pub fn change_password(&self, new_password: &SecretInput) -> CoreResult<()> {
        let pw = new_password.bytes();
        self.with(|v| v.change_password(&pw, Argon2Cost::DEFAULT))
    }

    /// Hex id of the vault (same as `VaultInfo.vault_id`).
    #[frb(sync, getter)]
    pub fn vault_id(&self) -> String {
        self.vault_id.clone()
    }

    /// Turns on biometric unlock for this device. Returns the 32-byte secret
    /// the platform keystore must protect; the caller wipes it after use.
    pub fn enroll_device(&self, device_id: String, label: String) -> CoreResult<Vec<u8>> {
        let device = parse_device(&device_id)?;
        self.with(|v| v.enroll_device(&device, &label).map(|s| s.to_vec()))
    }

    pub fn remove_device(&self, device_id: String) -> CoreResult<()> {
        let device = parse_device(&device_id)?;
        self.with(|v| v.remove_device(&device))
    }

    /// When biometric unlock on this device stops working (Unix seconds), or
    /// None when it is off.
    pub fn device_expires(&self, device_id: String) -> CoreResult<Option<i64>> {
        let device = parse_device(&device_id)?;
        self.with(|v| Ok(v.device_expires(&device)))
    }

    /// Replaces the recovery key; the old phrase stops working.
    pub fn new_recovery_phrase(&self) -> CoreResult<String> {
        self.with(|v| v.new_recovery_key().map(|p| p.to_string()))
    }

    /// Seals items into one `.atropos` file. With a passphrase anyone who
    /// knows it can open the package; without, only this vault can.
    pub fn seal_package(
        &self,
        ids: Vec<String>,
        out_file: String,
        passphrase: Option<SecretInput>,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let ids = parse_ids(&ids)?;
        let pw = passphrase.as_ref().map(|p| p.bytes());
        let o = self.with(|v| {
            let to = match &pw {
                Some(pw) => SealTo::Passphrase(pw),
                None => SealTo::vault(v)?,
            };
            package::create(
                Path::new(&out_file),
                to,
                &[],
                Some((v, &ids)),
                &mut reporter(&sink),
            )
        })?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    /// Imports a package into this vault. Vault-locked packages need no
    /// passphrase when they were sealed to this vault.
    pub fn open_package(
        &self,
        package_file: String,
        passphrase: Option<SecretInput>,
        parent: Option<String>,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let parent = parse_parent(parent)?;
        let pw = passphrase.as_ref().map(|p| p.bytes());
        let path = PathBuf::from(&package_file);
        // The vault is both the key and the destination; open a second
        // handle for the key so the borrow rules stay simple.
        let o = self.with(|v| match &pw {
            Some(pw) => package::open(
                &path,
                PackageKey::Passphrase(pw),
                Destination::Vault { vault: v, parent },
                &mut reporter(&sink),
            ),
            None => {
                let key_vault = v.key_clone()?;
                package::open(
                    &path,
                    PackageKey::Vault(&key_vault),
                    Destination::Vault { vault: v, parent },
                    &mut reporter(&sink),
                )
            }
        })?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    /// Opens a package locked to this vault into a folder on disk.
    pub fn extract_package(
        &self,
        package_file: String,
        dest_dir: String,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let o = self.with(|v| {
            package::open(
                Path::new(&package_file),
                PackageKey::Vault(v),
                Destination::Folder(Path::new(&dest_dir)),
                &mut reporter(&sink),
            )
        })?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }
}

/// How a package file is locked, read without opening it.
pub fn inspect_package(path: String) -> CoreResult<PackageInfo> {
    Ok(match package::inspect(Path::new(&path))? {
        PackageLock::Passphrase => PackageInfo {
            needs_passphrase: true,
            vault_id: String::new(),
        },
        PackageLock::Vault { vault_id } => PackageInfo {
            needs_passphrase: false,
            vault_id: hex::encode(vault_id),
        },
    })
}

/// Seals files and folders from disk into a passphrase-locked package.
pub fn seal_files(
    sources: Vec<String>,
    out_file: String,
    passphrase: &SecretInput,
    sink: StreamSink<TaskEvent>,
) -> CoreResult<()> {
    let pw = passphrase.bytes();
    let sources: Vec<PathBuf> = sources.into_iter().map(PathBuf::from).collect();
    let o = package::create(
        Path::new(&out_file),
        SealTo::Passphrase(&pw),
        &sources,
        None,
        &mut reporter(&sink),
    )?;
    let _ = sink.add(TaskEvent::Done(outcome_info(o)));
    Ok(())
}

/// Opens a passphrase-locked package into a folder on disk.
pub fn extract_files(
    package_file: String,
    passphrase: &SecretInput,
    dest_dir: String,
    sink: StreamSink<TaskEvent>,
) -> CoreResult<()> {
    let pw = passphrase.bytes();
    let o = package::open(
        Path::new(&package_file),
        PackageKey::Passphrase(&pw),
        Destination::Folder(Path::new(&dest_dir)),
        &mut reporter(&sink),
    )
    .map_err(wrong_secret)?;
    let _ = sink.add(TaskEvent::Done(outcome_info(o)));
    Ok(())
}
