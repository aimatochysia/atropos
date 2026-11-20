//! Browsing a package in place: its files are listed and viewed straight
//! from the encrypted `.atropos` file, without writing anything to disk.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use super::secure_input::SecretInput;
use super::vault::{wrong_secret, EntryInfo, EntryKind, VaultSession};
use super::viewer::{FileViewer, Viewers};
use crate::error::{CoreError, CoreResult};
use crate::vault::manifest::Kind;
use crate::vault::package::{PackageKey, PackageReader};
use crate::view;

struct Inner {
    reader: Mutex<Option<PackageReader>>,
    viewers: Viewers,
}

impl Inner {
    fn close(&self) {
        let viewers = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        for v in viewers.iter().filter_map(Weak::upgrade) {
            *v.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        drop(viewers);
        *self.reader.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn is_closed(&self) -> bool {
        self.reader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }
}

/// Every open package, so locking the app closes them all.
static OPEN: Mutex<Vec<Weak<Inner>>> = Mutex::new(Vec::new());

/// An opened package. Its key and any open files are wiped by
/// [PackageSession::close] or [close_all_packages].
#[frb(opaque)]
pub struct PackageSession {
    inner: Arc<Inner>,
    name: String,
}

fn closed() -> CoreError {
    CoreError::InvalidInput {
        message: "the package was closed".into(),
    }
}

fn session(path: &str, reader: PackageReader) -> PackageSession {
    let inner = Arc::new(Inner {
        reader: Mutex::new(Some(reader)),
        viewers: Arc::new(Mutex::new(Vec::new())),
    });
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    open.retain(|w| w.strong_count() > 0);
    open.push(Arc::downgrade(&inner));
    let name = Path::new(path)
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    PackageSession { inner, name }
}

/// Opens a passphrase-locked package for browsing.
pub fn browse_package(path: String, passphrase: &SecretInput) -> CoreResult<PackageSession> {
    let pw = passphrase.bytes();
    let reader =
        PackageReader::open(Path::new(&path), PackageKey::Passphrase(&pw)).map_err(wrong_secret)?;
    Ok(session(&path, reader))
}

/// Closes every open package (called when the app locks).
#[frb(sync)]
pub fn close_all_packages() {
    let open = std::mem::take(&mut *OPEN.lock().unwrap_or_else(|e| e.into_inner()));
    for p in open.iter().filter_map(Weak::upgrade) {
        p.close();
    }
}

impl VaultSession {
    /// Opens a package locked to this vault for browsing.
    pub fn browse_package(&self, path: String) -> CoreResult<PackageSession> {
        let reader = self.with(|v| PackageReader::open(Path::new(&path), PackageKey::Vault(v)))?;
        Ok(session(&path, reader))
    }
}

fn parse_index(id: &str) -> CoreResult<usize> {
    id.parse().map_err(|_| CoreError::InvalidInput {
        message: "that item is not in the package".into(),
    })
}

impl PackageSession {
    fn with<T>(&self, f: impl FnOnce(&mut PackageReader) -> CoreResult<T>) -> CoreResult<T> {
        let mut g = self.inner.reader.lock().unwrap_or_else(|e| e.into_inner());
        f(g.as_mut().ok_or_else(closed)?)
    }

    /// The package file's name without `.atropos`.
    #[frb(sync, getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[frb(sync)]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Items directly inside `parent` (None = the top level), folders first.
    pub fn list(&self, parent: Option<String>) -> CoreResult<Vec<EntryInfo>> {
        let parent = parent.map(|p| parse_index(&p)).transpose()?;
        self.with(|r| {
            let items = r.items();
            let base: &[String] = match parent {
                Some(i) => &items.get(i).ok_or_else(closed)?.path,
                None => &[],
            };
            let mut counts: HashMap<&[String], u32> = HashMap::new();
            for it in items {
                if let Some((_, up)) = it.path.split_last() {
                    *counts.entry(up).or_default() += 1;
                }
            }
            let mut out: Vec<EntryInfo> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.path.len() == base.len() + 1 && it.path.starts_with(base))
                .map(|(i, it)| EntryInfo {
                    id: i.to_string(),
                    name: it.path.last().cloned().unwrap_or_default(),
                    kind: match it.kind {
                        Kind::File => EntryKind::File,
                        Kind::Dir => EntryKind::Folder,
                        Kind::Symlink => EntryKind::Link,
                    },
                    size: it.size,
                    modified: it.modified,
                    executable: it.executable,
                    children: counts.get(it.path.as_slice()).copied().unwrap_or(0),
                })
                .collect();
            out.sort_by(|a, b| {
                (a.kind != EntryKind::Folder)
                    .cmp(&(b.kind != EntryKind::Folder))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            Ok(out)
        })
    }

    /// Decrypts one file of the package into memory for the viewer.
    pub fn open_viewer(&self, id: String) -> CoreResult<FileViewer> {
        let index = parse_index(&id)?;
        let (name, data) = self.with(|r| {
            let item = r.items().get(index).cloned().ok_or_else(closed)?;
            if item.size > view::MAX_VIEW_BYTES {
                return Err(CoreError::InvalidInput {
                    message: "this file is too large for the viewer".into(),
                });
            }
            let mut buf = Zeroizing::new(Vec::with_capacity(item.size as usize));
            r.read_file(index, &mut *buf)?;
            Ok((item.path.last().cloned().unwrap_or_default(), buf))
        })?;
        let inner = self.inner.clone();
        FileViewer::new(name, data, &self.inner.viewers, move || inner.is_closed())
    }

    /// Wipes the package key and every file opened from it.
    #[frb(sync)]
    pub fn close(&self) {
        self.inner.close();
    }
}
