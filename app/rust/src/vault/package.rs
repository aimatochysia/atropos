//! Sealed packages: one `.atropos` file holding files and folders, for
//! uploading, emailing or carrying on a USB stick.
//!
//! ```text
//! "ATROPOS-PKG" 0x01 | header length (u32 BE) | header JSON
//! | STREAM(AES-256-GCM, 64 KiB chunks), associated data = everything before it
//!   plaintext: index length (u64 BE) | index JSON | file bytes in index order
//!              | zero padding to a Padmé length
//! ```
//!
//! A package is locked either with a passphrase (Argon2id) or to a vault's
//! public key (hybrid ML-KEM-1024 + P-384), so only that vault can open it.
//! Names, folder structure and exact sizes are all inside the ciphertext.
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::header::CostJson;
use super::manifest::{Entry, Id, Kind};
use super::{
    clean_name, file_executable, file_mtime, set_mtime, unique_disk_name, Outcome, Pack, Progress,
    Vault,
};
use crate::crypto::hybrid::{self, HybridCiphertext, HybridPublicKey};
use crate::crypto::password::{self, Argon2Cost};
use crate::crypto::stream::{StreamReader, StreamWriter, PREFIX_LEN};
use crate::crypto::{padme, random_array};
use crate::error::{CoreError, CoreResult};

const MAGIC: &[u8; 12] = b"ATROPOS-PKG\x01";
const MAX_HEADER: u32 = 64 * 1024;
const MAX_INDEX: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
enum Lock {
    Passphrase {
        argon2: CostJson,
        #[serde(with = "super::b64")]
        salt: Vec<u8>,
    },
    Vault {
        #[serde(with = "super::b64")]
        vault_id: Vec<u8>,
        #[serde(with = "super::b64")]
        mlkem: Vec<u8>,
        #[serde(with = "super::b64")]
        p384: Vec<u8>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Header {
    version: u32,
    lock: Lock,
    #[serde(with = "super::b64")]
    prefix: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Item {
    /// Path inside the package, one name per level.
    path: Vec<String>,
    kind: Kind,
    size: u64,
    modified: i64,
    #[serde(default)]
    executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link_target: Option<String>,
}

/// What is known about a package before it is opened.
pub enum PackageLock {
    Passphrase,
    Vault { vault_id: Vec<u8> },
}

/// Who can open a new package.
pub enum SealTo<'a> {
    Passphrase(&'a [u8]),
    Vault {
        vault_id: Vec<u8>,
        public: HybridPublicKey,
    },
}

impl SealTo<'_> {
    pub fn vault(v: &Vault) -> CoreResult<SealTo<'static>> {
        Ok(SealTo::Vault {
            vault_id: v.vault_id().to_vec(),
            public: v.public_key()?,
        })
    }
}

/// How to open a package.
pub enum PackageKey<'a> {
    Passphrase(&'a [u8]),
    Vault(&'a Vault),
}

enum Source {
    Disk(PathBuf),
    Vault(Id),
}

fn header_bytes(h: &Header) -> CoreResult<Vec<u8>> {
    Ok(serde_json::to_vec(h)?)
}

fn read_header(r: &mut impl Read) -> CoreResult<(Header, Vec<u8>)> {
    let not_pkg = || CoreError::InvalidInput {
        message: "this is not an Atropos package, or it is damaged".into(),
    };
    let mut magic = [0u8; 12];
    r.read_exact(&mut magic).map_err(|_| not_pkg())?;
    if &magic != MAGIC {
        return Err(not_pkg());
    }
    let mut len = [0u8; 4];
    r.read_exact(&mut len).map_err(|_| not_pkg())?;
    let len = u32::from_be_bytes(len);
    if len > MAX_HEADER {
        return Err(not_pkg());
    }
    let mut hb = vec![0u8; len as usize];
    r.read_exact(&mut hb).map_err(|_| not_pkg())?;
    let h: Header = serde_json::from_slice(&hb).map_err(|_| not_pkg())?;
    if h.version != 1 || h.prefix.len() != PREFIX_LEN {
        return Err(CoreError::InvalidInput {
            message: "this package was made by a newer version of Atropos".into(),
        });
    }
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&len.to_be_bytes());
    aad.extend_from_slice(&hb);
    Ok((h, aad))
}

/// Says how a package is locked, without opening it.
pub fn inspect(path: &Path) -> CoreResult<PackageLock> {
    let mut f = BufReader::new(fs::File::open(path)?);
    let (h, _) = read_header(&mut f)?;
    Ok(match h.lock {
        Lock::Passphrase { .. } => PackageLock::Passphrase,
        Lock::Vault { vault_id, .. } => PackageLock::Vault { vault_id },
    })
}

fn collect_disk(
    src: &Path,
    base: &[String],
    items: &mut Vec<(Item, Source)>,
    p: &mut Progress,
) -> CoreResult<()> {
    let meta = fs::symlink_metadata(src)?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".into());
    let mut path = base.to_vec();
    path.push(name);
    let mut item = Item {
        path: path.clone(),
        kind: Kind::File,
        size: 0,
        modified: file_mtime(&meta),
        executable: file_executable(&meta),
        link_target: None,
    };
    if meta.file_type().is_symlink() {
        item.kind = Kind::Symlink;
        item.link_target = Some(fs::read_link(src)?.to_string_lossy().to_string());
        items.push((item, Source::Disk(src.to_path_buf())));
    } else if meta.is_dir() {
        item.kind = Kind::Dir;
        items.push((item, Source::Disk(src.to_path_buf())));
        let mut children: Vec<PathBuf> = fs::read_dir(src)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        children.sort();
        for c in children {
            collect_disk(&c, &path, items, p)?;
        }
    } else if meta.is_file() {
        item.size = meta.len();
        p.files_total += 1;
        p.bytes_total += item.size;
        items.push((item, Source::Disk(src.to_path_buf())));
    }
    Ok(())
}

fn collect_vault(
    v: &Vault,
    id: Id,
    base: &[String],
    items: &mut Vec<(Item, Source)>,
    p: &mut Progress,
) -> CoreResult<()> {
    let e = v
        .manifest()
        .get(&id)
        .cloned()
        .ok_or(CoreError::InvalidInput {
            message: "that item no longer exists".into(),
        })?;
    let mut path = base.to_vec();
    path.push(e.name.clone());
    items.push((
        Item {
            path: path.clone(),
            kind: e.kind,
            size: e.size,
            modified: e.modified,
            executable: e.executable,
            link_target: e.link_target.clone(),
        },
        Source::Vault(id),
    ));
    if e.kind == Kind::File {
        p.files_total += 1;
        p.bytes_total += e.size;
    }
    let children: Vec<Id> = v
        .manifest()
        .children(Some(id))
        .iter()
        .map(|c| c.id)
        .collect();
    for c in children {
        collect_vault(v, c, &path, items, p)?;
    }
    Ok(())
}

/// Seals files from disk (`disk`) and/or from an unlocked vault (`from_vault`
/// with `vault_items`) into the package file `out`. With no sources at all
/// it makes an empty package (a new single-file vault).
pub fn create(
    out: &Path,
    key: SealTo,
    disk: &[PathBuf],
    mut from_vault: Option<(&mut Vault, &[Id])>,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<Outcome> {
    let mut p = Progress::default();
    let mut items = Vec::new();
    for d in disk {
        collect_disk(d, &[], &mut items, &mut p)?;
    }
    if let Some((v, ids)) = from_vault.as_mut() {
        for id in ids.iter() {
            collect_vault(v, *id, &[], &mut items, &mut p)?;
        }
    }
    if items.is_empty() && !(disk.is_empty() && from_vault.is_none()) {
        return Err(CoreError::InvalidInput {
            message: "nothing to put in the package".into(),
        });
    }
    progress(&p);

    let prefix = random_array::<PREFIX_LEN>()?;
    let (lock, stream_key): (Lock, Zeroizing<[u8; 32]>) = match &key {
        SealTo::Passphrase(pw) => {
            crate::api::secure_input::check_strength(pw)?;
            let salt = random_array::<{ password::SALT_LEN }>()?;
            let cost = Argon2Cost::DEFAULT;
            let k = password::derive(pw, &salt, cost)?;
            (
                Lock::Passphrase {
                    argon2: cost.into(),
                    salt: salt.to_vec(),
                },
                k,
            )
        }
        SealTo::Vault { vault_id, public } => {
            let (ct, k) = hybrid::encapsulate(public)?;
            (
                Lock::Vault {
                    vault_id: vault_id.clone(),
                    mlkem: ct.mlkem,
                    p384: ct.p384_ephemeral,
                },
                k,
            )
        }
    };
    let header = Header {
        version: 1,
        lock,
        prefix: prefix.to_vec(),
    };
    let hb = header_bytes(&header)?;
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&(hb.len() as u32).to_be_bytes());
    aad.extend_from_slice(&hb);

    let index: Vec<Item> = items.iter().map(|(i, _)| i.clone()).collect();
    let index_json = Zeroizing::new(serde_json::to_vec(&index)?);
    let data_len: u64 = items
        .iter()
        .filter(|(i, _)| i.kind == Kind::File)
        .map(|(i, _)| i.size)
        .sum();
    let plain_len = 8 + index_json.len() as u64 + data_len;

    let tmp = out.with_extension("atropos-part");
    let result = (|| -> CoreResult<Outcome> {
        let mut file = BufWriter::new(fs::File::create(&tmp)?);
        file.write_all(&aad)?;
        let mut w = StreamWriter::new(file, &stream_key, prefix, &aad)?;
        w.write_all(&(index_json.len() as u64).to_be_bytes())?;
        w.write_all(&index_json)?;
        let mut outcome = Outcome::default();
        for (item, src) in &items {
            match item.kind {
                Kind::Dir => outcome.folders += 1,
                Kind::Symlink => outcome.files += 1,
                Kind::File => {
                    p.current = item.path.last().cloned().unwrap_or_default();
                    let before = w.written;
                    match src {
                        Source::Disk(path) => {
                            let mut f = fs::File::open(path)?.take(item.size + 1);
                            std::io::copy(&mut f, &mut w)?;
                        }
                        Source::Vault(id) => {
                            let v = from_vault
                                .as_mut()
                                .map(|(v, _)| &mut **v)
                                .ok_or(CoreError::Crypto)?;
                            v.read_file(*id, &mut w)?;
                        }
                    }
                    if w.written - before != item.size {
                        return Err(CoreError::InvalidInput {
                            message: format!(
                                "{} changed while it was being packed; try again",
                                p.current
                            ),
                        });
                    }
                    outcome.files += 1;
                    outcome.bytes += item.size;
                    p.files_done += 1;
                    p.bytes_done += item.size;
                    progress(&p);
                }
            }
        }
        let pad = padme(plain_len.max(4096)) - plain_len;
        std::io::copy(&mut std::io::repeat(0).take(pad), &mut w)?;
        let file = w.finish()?;
        file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(outcome)
    })();
    match result {
        Ok(o) => {
            fs::rename(&tmp, out)?;
            Ok(o)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Where an opened package goes.
pub enum Destination<'a> {
    Folder(&'a Path),
    Vault {
        vault: &'a mut Vault,
        parent: Option<Id>,
    },
}

fn stream_key(h: &Header, key: &PackageKey) -> CoreResult<Zeroizing<[u8; 32]>> {
    match (&h.lock, key) {
        (Lock::Passphrase { argon2, salt }, PackageKey::Passphrase(pw)) => {
            let salt: [u8; password::SALT_LEN] = salt
                .as_slice()
                .try_into()
                .map_err(|_| CoreError::DecryptFailed)?;
            password::derive(pw, &salt, (*argon2).into())
        }
        (
            Lock::Vault {
                vault_id,
                mlkem,
                p384,
            },
            PackageKey::Vault(v),
        ) => {
            if vault_id.as_slice() != v.vault_id() {
                return Err(CoreError::InvalidInput {
                    message: "this package is locked to a different vault".into(),
                });
            }
            v.decapsulate(&HybridCiphertext {
                mlkem: mlkem.clone(),
                p384_ephemeral: p384.clone(),
            })
        }
        (Lock::Passphrase { .. }, _) => Err(CoreError::InvalidInput {
            message: "this package needs its passphrase".into(),
        }),
        (Lock::Vault { .. }, _) => Err(CoreError::InvalidInput {
            message: "this package is locked to a vault; unlock that vault first".into(),
        }),
    }
}

fn validate(index: &[Item]) -> CoreResult<()> {
    let bad = || CoreError::InvalidInput {
        message: "the package's file list is invalid".into(),
    };
    for item in index {
        if item.path.is_empty() {
            return Err(bad());
        }
        for part in &item.path {
            if clean_name(part).ok().as_deref() != Some(part.as_str()) {
                return Err(bad());
            }
        }
    }
    Ok(())
}

/// Opens a package into a folder on disk or into an unlocked vault. On any
/// error everything it created is removed again.
pub fn open(
    path: &Path,
    key: PackageKey,
    dest: Destination,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<Outcome> {
    open_keyed(path, |h| stream_key(h, &key), dest, progress)
}

fn open_keyed(
    path: &Path,
    key: impl FnOnce(&Header) -> CoreResult<Zeroizing<[u8; 32]>>,
    dest: Destination,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<Outcome> {
    let mut file = BufReader::new(fs::File::open(path)?);
    let (header, aad) = read_header(&mut file)?;
    let k = key(&header)?;
    let prefix: [u8; PREFIX_LEN] = header
        .prefix
        .as_slice()
        .try_into()
        .map_err(|_| CoreError::DecryptFailed)?;
    let mut r = StreamReader::new(file, &k, prefix, &aad)?;
    let wrong =
        |e: std::io::Error| match e.into_inner().and_then(|i| i.downcast::<CoreError>().ok()) {
            Some(c) => *c,
            None => CoreError::DecryptFailed,
        };
    let mut len = [0u8; 8];
    r.read_exact(&mut len).map_err(wrong)?;
    let len = u64::from_be_bytes(len);
    if len > MAX_INDEX {
        return Err(CoreError::DecryptFailed);
    }
    let mut ij = Zeroizing::new(vec![0u8; len as usize]);
    r.read_exact(&mut ij).map_err(wrong)?;
    let index: Vec<Item> = serde_json::from_slice(&ij).map_err(|_| CoreError::DecryptFailed)?;
    validate(&index)?;
    let mut p = Progress {
        files_total: index.iter().filter(|i| i.kind == Kind::File).count() as u64,
        bytes_total: index
            .iter()
            .filter(|i| i.kind == Kind::File)
            .map(|i| i.size)
            .sum(),
        ..Default::default()
    };
    progress(&p);

    match dest {
        Destination::Folder(root) => {
            fs::create_dir_all(root)?;
            let mut created: Vec<PathBuf> = Vec::new();
            let result = extract_to_folder(&index, &mut r, root, &mut created, &mut p, progress);
            let result = result.and_then(|o| finish(&mut r).map(|_| o));
            if result.is_err() {
                for c in created.iter().rev() {
                    let _ = fs::remove_file(c).or_else(|_| fs::remove_dir(c));
                }
            }
            result
        }
        Destination::Vault { vault, parent } => {
            let mut created: Vec<Id> = Vec::new();
            let result = extract_to_vault(
                &index,
                &mut r,
                vault,
                parent,
                &mut created,
                &mut p,
                progress,
            );
            let result = result.and_then(|o| finish(&mut r).map(|_| o));
            if result.is_err() {
                let _ = vault.delete(&created);
            }
            result
        }
    }
}

/// Reads the padding to the end, which also authenticates the final chunk;
/// a truncated package fails here.
fn finish<R: Read>(r: &mut StreamReader<R>) -> CoreResult<()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = r.read(&mut buf).map_err(|_| CoreError::DecryptFailed)?;
        if n == 0 {
            break;
        }
        if buf[..n].iter().any(|b| *b != 0) {
            return Err(CoreError::DecryptFailed);
        }
    }
    if r.at_end()? {
        Ok(())
    } else {
        Err(CoreError::DecryptFailed)
    }
}

fn extract_to_folder(
    index: &[Item],
    r: &mut impl Read,
    root: &Path,
    created: &mut Vec<PathBuf>,
    p: &mut Progress,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<Outcome> {
    let mut outcome = Outcome::default();
    // Map package paths to the (possibly renamed) disk paths.
    let mut placed: std::collections::HashMap<Vec<String>, PathBuf> =
        std::collections::HashMap::new();
    for item in index {
        let parent_dir = if item.path.len() == 1 {
            root.to_path_buf()
        } else {
            placed
                .get(&item.path[..item.path.len() - 1])
                .cloned()
                .ok_or(CoreError::DecryptFailed)?
        };
        let name = item.path.last().ok_or(CoreError::DecryptFailed)?;
        let target = unique_disk_name(&parent_dir, name);
        p.current = name.clone();
        match item.kind {
            Kind::Dir => {
                fs::create_dir_all(&target)?;
                created.push(target.clone());
                placed.insert(item.path.clone(), target);
                outcome.folders += 1;
            }
            Kind::Symlink => {
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(
                        item.link_target.clone().unwrap_or_default(),
                        &target,
                    )?;
                    created.push(target);
                }
                #[cfg(not(unix))]
                outcome.skipped.push(format!("{name} (symbolic link)"));
                outcome.files += 1;
            }
            Kind::File => {
                let mut f = BufWriter::new(fs::File::create(&target)?);
                created.push(target.clone());
                let copied = std::io::copy(&mut r.take(item.size), &mut f)
                    .map_err(|_| CoreError::DecryptFailed)?;
                if copied != item.size {
                    return Err(CoreError::DecryptFailed);
                }
                f.into_inner().map_err(|e| e.into_error())?.sync_all()?;
                set_mtime(&target, item.modified);
                #[cfg(unix)]
                if item.executable {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perm = fs::metadata(&target)?.permissions();
                    perm.set_mode(perm.mode() | 0o755);
                    fs::set_permissions(&target, perm)?;
                }
                outcome.files += 1;
                outcome.bytes += item.size;
                p.files_done += 1;
                p.bytes_done += item.size;
                progress(p);
            }
        }
    }
    Ok(outcome)
}

fn extract_to_vault(
    index: &[Item],
    r: &mut impl Read,
    vault: &mut Vault,
    parent: Option<Id>,
    created: &mut Vec<Id>,
    p: &mut Progress,
    progress: &mut dyn FnMut(&Progress),
) -> CoreResult<Outcome> {
    let mut outcome = Outcome::default();
    let mut placed: std::collections::HashMap<Vec<String>, Id> = std::collections::HashMap::new();
    let mut pack = Pack::new();
    for item in index {
        let item_parent = if item.path.len() == 1 {
            parent
        } else {
            Some(
                *placed
                    .get(&item.path[..item.path.len() - 1])
                    .ok_or(CoreError::DecryptFailed)?,
            )
        };
        let name = item.path.last().ok_or(CoreError::DecryptFailed)?;
        let id = random_array::<16>()?;
        let entry = Entry {
            id,
            parent: item_parent,
            name: vault.unique_pending_name(item_parent, name, &pack),
            kind: item.kind,
            size: if item.kind == Kind::File {
                item.size
            } else {
                0
            },
            modified: item.modified,
            executable: item.executable && item.kind == Kind::File,
            link_target: item.link_target.clone(),
            data: None,
            sha256: None,
            open_as: None,
        };
        p.current = name.clone();
        created.push(id);
        match item.kind {
            Kind::Dir => {
                vault.put_entry(entry)?;
                placed.insert(item.path.clone(), id);
                outcome.folders += 1;
            }
            Kind::Symlink => {
                vault.put_entry(entry)?;
                outcome.files += 1;
            }
            Kind::File => {
                let mut limited = r.take(item.size);
                vault.add_file(entry, &mut limited, &mut pack)?;
                outcome.files += 1;
                outcome.bytes += item.size;
                p.files_done += 1;
                p.bytes_done += item.size;
                progress(p);
            }
        }
    }
    vault.flush_pack(&mut pack)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::header::Unlock;

    const PW: &[u8] = b"violet-harbor-tundra-9";

    fn sample(root: &Path) -> PathBuf {
        let d = root.join("photos");
        fs::create_dir_all(d.join("2026")).unwrap();
        fs::write(d.join("2026/a.jpg"), vec![1u8; 70_000]).unwrap();
        fs::write(d.join("note.txt"), "secret note").unwrap();
        d
    }

    #[test]
    fn passphrase_package_round_trip_and_tamper() {
        let tmp = tempfile::tempdir().unwrap();
        let src = sample(tmp.path());
        let pkg = tmp.path().join("photos.atropos");
        create(
            &pkg,
            SealTo::Passphrase(PW),
            std::slice::from_ref(&src),
            None,
            &mut |_| {},
        )
        .unwrap();
        let bytes = fs::read(&pkg).unwrap();
        assert!(!bytes.windows(11).any(|w| w == b"secret note"));
        assert!(!bytes.windows(8).any(|w| w == b"note.txt"));
        assert!(matches!(inspect(&pkg).unwrap(), PackageLock::Passphrase));

        let out = tmp.path().join("out");
        open(
            &pkg,
            PackageKey::Passphrase(PW),
            Destination::Folder(&out),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(out.join("photos/note.txt")).unwrap(),
            b"secret note"
        );
        assert_eq!(
            fs::read(out.join("photos/2026/a.jpg")).unwrap(),
            vec![1u8; 70_000]
        );

        assert!(open(
            &pkg,
            PackageKey::Passphrase(b"wrong-password-xyz-1"),
            Destination::Folder(&tmp.path().join("x")),
            &mut |_| {}
        )
        .is_err());

        // Truncated: nothing is left behind.
        let cut = tmp.path().join("cut.atropos");
        fs::write(&cut, &bytes[..bytes.len() - 100]).unwrap();
        let out2 = tmp.path().join("out2");
        assert!(open(
            &cut,
            PackageKey::Passphrase(PW),
            Destination::Folder(&out2),
            &mut |_| {}
        )
        .is_err());
        assert!(fs::read_dir(&out2)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true));
    }

    #[test]
    fn vault_locked_package_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = sample(tmp.path());
        let (mut v, _) = Vault::create(&tmp.path().join("v"), PW, Argon2Cost::MINIMUM).unwrap();
        v.import(&[src], None, false, &mut |_| {}).unwrap();
        let ids: Vec<Id> = v.manifest().children(None).iter().map(|e| e.id).collect();
        let pkg = tmp.path().join("p.atropos");
        let to = SealTo::vault(&v).unwrap();
        create(&pkg, to, &[], Some((&mut v, &ids)), &mut |_| {}).unwrap();

        // A second vault cannot open it.
        let (other, _) = Vault::create(&tmp.path().join("w"), PW, Argon2Cost::MINIMUM).unwrap();
        assert!(open(
            &pkg,
            PackageKey::Vault(&other),
            Destination::Folder(&tmp.path().join("o")),
            &mut |_| {}
        )
        .is_err());
        drop(other);

        // Import it back into the same vault: a renamed copy appears.
        let path = v.path().to_path_buf();
        drop(v);
        let v = Vault::unlock(&path, &Unlock::Password(PW)).unwrap();
        let mut v2 = Vault::unlock(&path, &Unlock::Password(PW)).unwrap();
        open(
            &pkg,
            PackageKey::Vault(&v),
            Destination::Vault {
                vault: &mut v2,
                parent: None,
            },
            &mut |_| {},
        )
        .unwrap();
        let names: Vec<String> = v2
            .manifest()
            .children(None)
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(names, vec!["photos", "photos (2)"]);
    }
}

/// Where a rewritten file's bytes come from.
enum Fill {
    /// An item of the package as it is now.
    Keep(usize),
    Bytes(Zeroizing<Vec<u8>>),
    Disk(PathBuf),
    /// Folders and links have no bytes.
    None,
}

impl Fill {
    fn again(&self) -> Fill {
        match self {
            Fill::Keep(i) => Fill::Keep(*i),
            Fill::Bytes(b) => Fill::Bytes(b.clone()),
            Fill::Disk(p) => Fill::Disk(p.clone()),
            Fill::None => Fill::None,
        }
    }
}

fn gone() -> CoreError {
    CoreError::InvalidInput {
        message: "that item is not in the package".into(),
    }
}

fn child(parent: &[String], name: &str) -> Vec<String> {
    let mut p = parent.to_vec();
    p.push(name.to_string());
    p
}

/// `path` with its leading `old` replaced by `new`.
fn rebase(path: &[String], old: &[String], new: &[String]) -> Vec<String> {
    let mut p = new.to_vec();
    p.extend_from_slice(&path[old.len()..]);
    p
}

fn new_item(path: Vec<String>, kind: Kind, size: u64) -> Item {
    Item {
        path,
        kind,
        size,
        modified: super::now(),
        executable: false,
        link_target: None,
    }
}

/// `parent` must be the top level or a folder of the package.
fn check_folder(plan: &[(Item, Fill)], parent: &[String]) -> CoreResult<()> {
    if parent.is_empty()
        || plan
            .iter()
            .any(|(i, _)| i.path == parent && i.kind == Kind::Dir)
    {
        Ok(())
    } else {
        Err(CoreError::InvalidInput {
            message: "that folder is not in the package".into(),
        })
    }
}

/// `wanted`, or `wanted (2)` and so on when `parent` already holds that
/// name (ignoring case, and ignoring the item at `ignore`).
fn free_name(
    plan: &[(Item, Fill)],
    parent: &[String],
    wanted: &str,
    ignore: Option<&[String]>,
) -> String {
    let taken = |n: &str| {
        plan.iter().any(|(i, _)| {
            i.path.len() == parent.len() + 1
                && i.path.starts_with(parent)
                && Some(i.path.as_slice()) != ignore
                && i.path.last().is_some_and(|l| l.eq_ignore_ascii_case(n))
        })
    };
    if !taken(wanted) {
        return wanted.to_string();
    }
    let (stem, ext) = match wanted.rfind('.') {
        Some(i) if i > 0 => (&wanted[..i], &wanted[i..]),
        _ => (wanted, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| wanted.to_string())
}

/// One item of an opened package, for browsing.
#[derive(Clone, Debug)]
pub struct PackageItem {
    pub path: Vec<String>,
    pub kind: Kind,
    pub size: u64,
    pub modified: i64,
    pub executable: bool,
}

/// A package opened for browsing in place: the file list is decrypted once,
/// and each file is decrypted chunk by chunk straight from the package
/// when it is viewed. Nothing is written anywhere.
pub struct PackageReader {
    path: PathBuf,
    header: Header,
    raw_key: Zeroizing<[u8; 32]>,
    index: Vec<Item>,
    file: fs::File,
    key: crate::crypto::aead::AeadKey,
    prefix: [u8; PREFIX_LEN],
    aad: Vec<u8>,
    /// Where the encrypted stream starts in the file.
    start: u64,
    /// Length of the encrypted stream.
    sealed: u64,
    items: Vec<PackageItem>,
    /// Plaintext offset of each item's bytes (0 for folders and links).
    offsets: Vec<u64>,
}

impl PackageReader {
    /// Opens `path` for browsing. Checks the final chunk too, so a cut-off
    /// package is caught here rather than on some later file.
    pub fn open(path: &Path, key: PackageKey) -> CoreResult<Self> {
        Self::open_inner(path, |h| stream_key(h, &key))
    }

    fn open_inner(
        path: &Path,
        key: impl FnOnce(&Header) -> CoreResult<Zeroizing<[u8; 32]>>,
    ) -> CoreResult<Self> {
        let mut file = fs::File::open(path)?;
        let (header, aad) = read_header(&mut BufReader::new(&mut file))?;
        let k = key(&header)?;
        let prefix: [u8; PREFIX_LEN] = header
            .prefix
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::DecryptFailed)?;
        let start = aad.len() as u64;
        let total = file.metadata()?.len();
        let sealed = total.checked_sub(start).ok_or(CoreError::DecryptFailed)?;
        let mut r = PackageReader {
            path: path.to_path_buf(),
            header,
            raw_key: k.clone(),
            index: Vec::new(),
            file,
            key: crate::crypto::aead::AeadKey::new(&k)?,
            prefix,
            aad,
            start,
            sealed,
            items: Vec::new(),
            offsets: Vec::new(),
        };
        // Authenticates the last chunk, and with it the stream's length.
        let chunks = crate::crypto::stream::chunk_count(sealed);
        r.chunk(chunks - 1)?;
        let mut len = Zeroizing::new(Vec::new());
        r.read_range(0, 8, &mut *len)?;
        let len = u64::from_be_bytes(len[..].try_into().map_err(|_| CoreError::DecryptFailed)?);
        if len > MAX_INDEX {
            return Err(CoreError::DecryptFailed);
        }
        let mut ij = Zeroizing::new(Vec::new());
        r.read_range(8, len, &mut *ij)?;
        let index: Vec<Item> = serde_json::from_slice(&ij).map_err(|_| CoreError::DecryptFailed)?;
        validate(&index)?;
        let mut off = 8 + len;
        r.index = index.clone();
        for item in index {
            r.offsets
                .push(if item.kind == Kind::File { off } else { 0 });
            if item.kind == Kind::File {
                off = off.checked_add(item.size).ok_or(CoreError::DecryptFailed)?;
            }
            r.items.push(PackageItem {
                path: item.path,
                kind: item.kind,
                size: item.size,
                modified: item.modified,
                executable: item.executable,
            });
        }
        // The files must fit inside the plaintext.
        let plain_len = sealed - chunks * crate::crypto::aead::TAG_LEN as u64;
        if off > plain_len {
            return Err(CoreError::DecryptFailed);
        }
        Ok(r)
    }

    pub fn items(&self) -> &[PackageItem] {
        &self.items
    }

    fn chunk(&mut self, index: u64) -> CoreResult<Zeroizing<Vec<u8>>> {
        use std::io::{Seek, SeekFrom};
        let sealed_chunk = (crate::crypto::stream::CHUNK_LEN + crate::crypto::aead::TAG_LEN) as u64;
        let chunks = crate::crypto::stream::chunk_count(self.sealed);
        if index >= chunks {
            return Err(CoreError::DecryptFailed);
        }
        let at = index * sealed_chunk;
        let size = sealed_chunk.min(self.sealed - at) as usize;
        let mut buf = vec![0u8; size];
        self.file.seek(SeekFrom::Start(self.start + at))?;
        self.file.read_exact(&mut buf)?;
        crate::crypto::stream::decrypt_chunk(
            &self.key,
            &self.prefix,
            &self.aad,
            index,
            index + 1 == chunks,
            &buf,
        )
    }

    /// Decrypts plaintext bytes `from..from + len` into `out`.
    fn read_range(&mut self, from: u64, len: u64, out: &mut dyn Write) -> CoreResult<()> {
        let chunk_len = crate::crypto::stream::CHUNK_LEN as u64;
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

    /// The index of the item at `path`.
    pub fn find(&self, path: &[String]) -> Option<usize> {
        self.items.iter().position(|i| i.path == path)
    }

    /// Writes the package again with file `index` holding `data`, under the
    /// same lock (passphrase or vault) and a fresh nonce prefix.
    pub fn replace_file(&mut self, index: usize, data: &[u8]) -> CoreResult<()> {
        if self.items.get(index).map(|i| i.kind) != Some(Kind::File) {
            return Err(CoreError::InvalidInput {
                message: "only files can be changed".into(),
            });
        }
        let mut plan = self.plan();
        plan[index].0.size = data.len() as u64;
        plan[index].0.modified = super::now();
        plan[index].1 = Fill::Bytes(Zeroizing::new(data.to_vec()));
        self.rewrite(plan, &mut |_| {})
    }

    /// Creates an empty folder `name` (a free name) inside `parent`.
    /// Returns its path.
    pub fn create_folder(&mut self, parent: &[String], name: &str) -> CoreResult<Vec<String>> {
        let mut plan = self.plan();
        check_folder(&plan, parent)?;
        let path = child(parent, &free_name(&plan, parent, &clean_name(name)?, None));
        plan.push((new_item(path.clone(), Kind::Dir, 0), Fill::None));
        self.rewrite(plan, &mut |_| {})?;
        Ok(path)
    }

    /// Creates file `name` (a free name) inside `parent`, holding `data`.
    /// Returns its path.
    pub fn create_file(
        &mut self,
        parent: &[String],
        name: &str,
        data: &[u8],
    ) -> CoreResult<Vec<String>> {
        let mut plan = self.plan();
        check_folder(&plan, parent)?;
        let path = child(parent, &free_name(&plan, parent, &clean_name(name)?, None));
        plan.push((
            new_item(path.clone(), Kind::File, data.len() as u64),
            Fill::Bytes(Zeroizing::new(data.to_vec())),
        ));
        self.rewrite(plan, &mut |_| {})?;
        Ok(path)
    }

    /// Adds files and folders from disk into `parent`.
    pub fn add_from_disk(
        &mut self,
        parent: &[String],
        sources: &[PathBuf],
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Outcome> {
        let mut plan = self.plan();
        check_folder(&plan, parent)?;
        let mut outcome = Outcome::default();
        for src in sources {
            let mut found = Vec::new();
            let mut p = Progress::default();
            collect_disk(src, parent, &mut found, &mut p)?;
            let Some((top, _)) = found.first() else {
                continue;
            };
            let old = top.path.clone();
            let name = old.last().cloned().unwrap_or_default();
            let name = free_name(&plan, parent, &clean_name(&name)?, None);
            let new = child(parent, &name);
            for (mut item, source) in found {
                item.path = rebase(&item.path, &old, &new);
                match item.kind {
                    Kind::Dir => outcome.folders += 1,
                    _ => {
                        outcome.files += 1;
                        outcome.bytes += item.size;
                    }
                }
                let fill = match source {
                    Source::Disk(path) if item.kind == Kind::File => Fill::Disk(path),
                    _ => Fill::None,
                };
                plan.push((item, fill));
            }
        }
        self.rewrite(plan, progress)?;
        Ok(outcome)
    }

    /// Renames the item at `path` (a folder with everything in it). Returns
    /// its new path.
    pub fn rename(&mut self, path: &[String], name: &str) -> CoreResult<Vec<String>> {
        let mut plan = self.plan();
        let (parent, _) = path.split_last().map(|(l, p)| (p, l)).ok_or_else(gone)?;
        if !plan.iter().any(|(i, _)| i.path == path) {
            return Err(gone());
        }
        let new = child(
            parent,
            &free_name(&plan, parent, &clean_name(name)?, Some(path)),
        );
        for (item, _) in plan.iter_mut() {
            if item.path.starts_with(path) {
                item.path = rebase(&item.path, path, &new);
            }
        }
        self.rewrite(plan, &mut |_| {})?;
        Ok(new)
    }

    /// Moves items (folders with everything in them) into `parent`.
    pub fn move_to(&mut self, paths: &[Vec<String>], parent: &[String]) -> CoreResult<()> {
        let mut plan = self.plan();
        check_folder(&plan, parent)?;
        for path in paths {
            if parent.starts_with(path) {
                return Err(CoreError::InvalidInput {
                    message: "a folder cannot be moved into itself".into(),
                });
            }
            if path.split_last().map(|(_, p)| p) == Some(parent) {
                continue;
            }
            let name = path.last().ok_or_else(gone)?;
            if !plan.iter().any(|(i, _)| &i.path == path) {
                return Err(gone());
            }
            let new = child(parent, &free_name(&plan, parent, name, None));
            // The moved items go to the end, after the folder they land in.
            let (mut moved, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut plan)
                .into_iter()
                .partition(|(i, _)| i.path.starts_with(path));
            for (item, _) in moved.iter_mut() {
                item.path = rebase(&item.path, path, &new);
            }
            plan = rest;
            plan.append(&mut moved);
        }
        self.rewrite(plan, &mut |_| {})
    }

    /// Copies items (folders with everything in them) into `parent`.
    pub fn copy_to(&mut self, paths: &[Vec<String>], parent: &[String]) -> CoreResult<()> {
        let mut plan = self.plan();
        check_folder(&plan, parent)?;
        for path in paths {
            if parent.starts_with(path) {
                return Err(CoreError::InvalidInput {
                    message: "a folder cannot be copied into itself".into(),
                });
            }
            let name = path.last().ok_or_else(gone)?;
            let copies: Vec<(Item, Fill)> = plan
                .iter()
                .filter(|(i, _)| i.path.starts_with(path))
                .map(|(i, f)| (i.clone(), f.again()))
                .collect();
            if copies.is_empty() {
                return Err(gone());
            }
            let new = child(parent, &free_name(&plan, parent, name, None));
            for (mut item, fill) in copies {
                item.path = rebase(&item.path, path, &new);
                plan.push((item, fill));
            }
        }
        self.rewrite(plan, &mut |_| {})
    }

    /// Deletes items (folders with everything in them).
    pub fn delete(&mut self, paths: &[Vec<String>]) -> CoreResult<()> {
        let mut plan = self.plan();
        plan.retain(|(i, _)| !paths.iter().any(|p| i.path.starts_with(p)));
        self.rewrite(plan, &mut |_| {})
    }

    /// Opens the whole package into `dest` with the key it is already open
    /// with.
    pub fn extract(
        &self,
        dest: Destination,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<Outcome> {
        let key = self.raw_key.clone();
        open_keyed(&self.path, |_| Ok(key), dest, progress)
    }

    /// The items as vault entries with fresh ids. A file's data offset is
    /// its index here, for reading it again with [PackageReader::read_file].
    pub(crate) fn vault_entries(&self) -> CoreResult<std::collections::BTreeMap<Id, Entry>> {
        use std::collections::{BTreeMap, HashMap};
        let mut ids: HashMap<&[String], Id> = HashMap::new();
        let mut out = BTreeMap::new();
        for (i, item) in self.index.iter().enumerate() {
            let id = random_array::<16>()?;
            let parent = match item.path.split_last() {
                Some((_, up)) if !up.is_empty() => {
                    Some(*ids.get(up).ok_or(CoreError::DecryptFailed)?)
                }
                _ => None,
            };
            ids.insert(&item.path, id);
            out.insert(
                id,
                Entry {
                    id,
                    parent,
                    name: item.path.last().cloned().unwrap_or_default(),
                    kind: item.kind,
                    size: item.size,
                    modified: item.modified,
                    executable: item.executable,
                    link_target: item.link_target.clone(),
                    data: (item.kind == Kind::File && item.size > 0).then_some(
                        super::manifest::DataRef {
                            blob: super::single::STREAM,
                            offset: i as u64,
                            len: item.size,
                        },
                    ),
                    sha256: None,
                    open_as: None,
                },
            );
        }
        Ok(out)
    }

    /// Every item as it is now, each keeping its own bytes.
    fn plan(&self) -> Vec<(Item, Fill)> {
        self.index
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, item)| {
                let fill = if item.kind == Kind::File {
                    Fill::Keep(i)
                } else {
                    Fill::None
                };
                (item, fill)
            })
            .collect()
    }

    /// Writes the package again with the items of `plan`, under the same
    /// lock (passphrase or vault) and a fresh nonce prefix. The new package
    /// is written beside the old one and then swapped in, so a crash leaves
    /// either the old or the new package, never half of one.
    fn rewrite(
        &mut self,
        plan: Vec<(Item, Fill)>,
        progress: &mut dyn FnMut(&Progress),
    ) -> CoreResult<()> {
        let new_index: Vec<Item> = plan.iter().map(|(i, _)| i.clone()).collect();
        validate(&new_index)?;
        let prefix = random_array::<PREFIX_LEN>()?;
        let header = Header {
            version: 1,
            lock: self.header.lock.clone(),
            prefix: prefix.to_vec(),
        };
        let hb = header_bytes(&header)?;
        let mut aad = MAGIC.to_vec();
        aad.extend_from_slice(&(hb.len() as u32).to_be_bytes());
        aad.extend_from_slice(&hb);
        let index_json = Zeroizing::new(serde_json::to_vec(&new_index)?);
        let files = new_index.iter().filter(|i| i.kind == Kind::File);
        let mut p = Progress {
            files_total: files.clone().count() as u64,
            bytes_total: files.map(|i| i.size).sum(),
            ..Default::default()
        };
        progress(&p);
        let plain_len = 8 + index_json.len() as u64 + p.bytes_total;
        let tmp = self.path.with_extension("atropos-part");
        let key = self.raw_key.clone();
        let result = (|| -> CoreResult<()> {
            let mut file = BufWriter::new(fs::File::create(&tmp)?);
            file.write_all(&aad)?;
            let mut w = StreamWriter::new(file, &key, prefix, &aad)?;
            w.write_all(&(index_json.len() as u64).to_be_bytes())?;
            w.write_all(&index_json)?;
            for (item, fill) in &plan {
                if item.kind != Kind::File {
                    continue;
                }
                p.current = item.path.last().cloned().unwrap_or_default();
                let before = w.written;
                match fill {
                    Fill::Keep(i) => self.read_file(*i, &mut w)?,
                    Fill::Bytes(b) => w.write_all(b)?,
                    Fill::Disk(path) => {
                        let mut f = fs::File::open(path)?.take(item.size + 1);
                        std::io::copy(&mut f, &mut w)?;
                    }
                    Fill::None => {}
                }
                if w.written - before != item.size {
                    return Err(CoreError::InvalidInput {
                        message: format!(
                            "{} changed while it was being packed; try again",
                            p.current
                        ),
                    });
                }
                p.files_done += 1;
                p.bytes_done += item.size;
                progress(&p);
            }
            let pad = padme(plain_len.max(4096)) - plain_len;
            std::io::copy(&mut std::io::repeat(0).take(pad), &mut w)?;
            let file = w.finish()?;
            file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        // Let go of the old file first (Windows will not replace an open one).
        self.file = fs::File::open(&tmp)?;
        if let Err(e) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            self.file = fs::File::open(&self.path)?;
            return Err(e.into());
        }
        // Read the new package back through the same key.
        let path = self.path.clone();
        *self = PackageReader::open_inner(&path, |_| Ok(key))?;
        Ok(())
    }

    /// Decrypts item `index` (a file) into `out`.
    pub fn read_file(&mut self, index: usize, out: &mut dyn Write) -> CoreResult<()> {
        let item = self.items.get(index).ok_or(CoreError::InvalidInput {
            message: "that item is not in the package".into(),
        })?;
        if item.kind != Kind::File {
            return Err(CoreError::InvalidInput {
                message: "only files can be opened".into(),
            });
        }
        let (from, size) = (self.offsets[index], item.size);
        self.read_range(from, size, out)
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    const PW: &[u8] = b"violet-harbor-tundra-9";

    #[test]
    fn browse_a_package_without_extracting() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("photos");
        fs::create_dir_all(d.join("2026")).unwrap();
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(d.join("2026/a.bin"), &big).unwrap();
        fs::write(d.join("note.txt"), "secret note").unwrap();
        let pkg = tmp.path().join("p.atropos");
        create(&pkg, SealTo::Passphrase(PW), &[d], None, &mut |_| {}).unwrap();

        let mut r = PackageReader::open(&pkg, PackageKey::Passphrase(PW)).unwrap();
        let find = |r: &PackageReader, name: &str| {
            r.items()
                .iter()
                .position(|i| i.path.last().map(String::as_str) == Some(name))
                .unwrap()
        };
        let a = find(&r, "a.bin");
        let n = find(&r, "note.txt");
        assert_eq!(r.items()[a].path, ["photos", "2026", "a.bin"]);
        let mut out = Vec::new();
        r.read_file(a, &mut out).unwrap();
        assert_eq!(out, big);
        out.clear();
        r.read_file(n, &mut out).unwrap();
        assert_eq!(out, b"secret note");
        assert!(r.read_file(find(&r, "2026"), &mut out).is_err());

        assert!(PackageReader::open(&pkg, PackageKey::Passphrase(b"wrong-pass-0000")).is_err());
        let bytes = fs::read(&pkg).unwrap();
        let cut = tmp.path().join("cut.atropos");
        fs::write(&cut, &bytes[..bytes.len() - 100]).unwrap();
        assert!(PackageReader::open(&cut, PackageKey::Passphrase(PW)).is_err());

        // Edit one file in place; the rest stays as it was.
        r.replace_file(n, b"secret note, edited").unwrap();
        let mut r = PackageReader::open(&pkg, PackageKey::Passphrase(PW)).unwrap();
        let n = find(&r, "note.txt");
        let mut out = Vec::new();
        r.read_file(n, &mut out).unwrap();
        assert_eq!(out, b"secret note, edited");
        out.clear();
        r.read_file(find(&r, "a.bin"), &mut out).unwrap();
        assert_eq!(out, big);
        assert!(!pkg.with_extension("atropos-part").exists());
        // Still opens with the same passphrase, and extracts.
        let dest = tmp.path().join("out");
        open(
            &pkg,
            PackageKey::Passphrase(PW),
            Destination::Folder(&dest),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(dest.join("photos/note.txt")).unwrap(),
            b"secret note, edited"
        );
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    const PW: &[u8] = b"violet-harbor-tundra-9";

    fn p(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn paths(r: &PackageReader) -> Vec<String> {
        r.items().iter().map(|i| i.path.join("/")).collect()
    }

    fn read(r: &mut PackageReader, path: &[&str]) -> Vec<u8> {
        let i = r.find(&p(path)).unwrap();
        let mut out = Vec::new();
        r.read_file(i, &mut out).unwrap();
        out
    }

    #[test]
    fn manage_files_inside_a_package() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("v.atropos");
        // An empty package is a new single-file vault.
        create(&pkg, SealTo::Passphrase(PW), &[], None, &mut |_| {}).unwrap();
        let mut r = PackageReader::open(&pkg, PackageKey::Passphrase(PW)).unwrap();
        assert!(r.items().is_empty());

        let docs = r.create_folder(&[], "docs").unwrap();
        assert_eq!(docs, p(&["docs"]));
        assert_eq!(r.create_folder(&[], "Docs").unwrap(), p(&["Docs (2)"]));
        r.create_file(&docs, "note.md", b"# hi").unwrap();
        assert_eq!(
            r.create_file(&docs, "note.md", b"two").unwrap(),
            p(&["docs", "note (2).md"])
        );
        assert!(r.create_file(&p(&["nope"]), "x", b"").is_err());
        assert!(r.create_file(&[], "a/b", b"").is_err());

        // Add from disk: a folder and a file, with a clashing name.
        let src = tmp.path().join("photos");
        fs::create_dir_all(src.join("2026")).unwrap();
        let big: Vec<u8> = (0..150_000u32).map(|i| (i % 249) as u8).collect();
        fs::write(src.join("2026/a.bin"), &big).unwrap();
        let loose = tmp.path().join("docs");
        fs::write(&loose, b"loose file").unwrap();
        let o = r
            .add_from_disk(&[], &[src.clone(), loose], &mut |_| {})
            .unwrap();
        assert_eq!((o.files, o.folders), (2, 2));
        assert!(paths(&r).contains(&"photos/2026/a.bin".to_string()));
        assert_eq!(read(&mut r, &["docs (3)"]), b"loose file");

        // Rename a folder: everything inside follows.
        let pics = r.rename(&p(&["photos"]), "pictures").unwrap();
        assert_eq!(read(&mut r, &["pictures", "2026", "a.bin"]), big);

        // Move into another folder, and refuse moving into itself.
        r.move_to(std::slice::from_ref(&pics), &docs).unwrap();
        assert_eq!(read(&mut r, &["docs", "pictures", "2026", "a.bin"]), big);
        assert!(r
            .move_to(std::slice::from_ref(&docs), &p(&["docs", "pictures"]))
            .is_err());
        // Parents always come before their children.
        let all = paths(&r);
        for (i, path) in all.iter().enumerate() {
            if let Some((up, _)) = path.rsplit_once('/') {
                assert!(all[..i].contains(&up.to_string()), "{all:?}");
            }
        }

        // Copy a folder next to itself.
        r.copy_to(std::slice::from_ref(&docs), &[]).unwrap();
        assert_eq!(read(&mut r, &["docs (4)", "note.md"]), b"# hi");
        assert_eq!(
            read(&mut r, &["docs (4)", "pictures", "2026", "a.bin"]),
            big
        );
        assert!(r.copy_to(std::slice::from_ref(&docs), &docs).is_err());

        // Delete, then reopen from disk and extract to check it all holds.
        r.delete(&[p(&["docs (4)"]), p(&["Docs (2)"])]).unwrap();
        drop(r);
        let mut r = PackageReader::open(&pkg, PackageKey::Passphrase(PW)).unwrap();
        assert_eq!(
            paths(&r),
            [
                "docs",
                "docs/note.md",
                "docs/note (2).md",
                "docs (3)",
                "docs/pictures",
                "docs/pictures/2026",
                "docs/pictures/2026/a.bin",
            ]
        );
        assert_eq!(read(&mut r, &["docs", "note (2).md"]), b"two");
        let out = tmp.path().join("out");
        r.extract(Destination::Folder(&out), &mut |_| {}).unwrap();
        assert_eq!(fs::read(out.join("docs/pictures/2026/a.bin")).unwrap(), big);
        assert!(!pkg.with_extension("atropos-part").exists());

        // Everything can go; the empty vault still opens.
        r.delete(&[p(&["docs"]), p(&["docs (3)"])]).unwrap();
        drop(r);
        let r = PackageReader::open(&pkg, PackageKey::Passphrase(PW)).unwrap();
        assert!(r.items().is_empty());
    }
}
