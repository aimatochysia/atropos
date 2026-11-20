//! Browsing a package in place: its files are listed and viewed straight
//! from the encrypted `.atropos` file, without writing anything to disk.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use super::secure_input::SecretInput;
use super::vault::{sort_entries, wrong_secret, EntryInfo, EntryKind, SortOrder, VaultSession};
use super::viewer::{FileViewer, ImageEdit, PreviewKind, Viewers};
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
    pub fn list(
        &self,
        parent: Option<String>,
        sort: Option<SortOrder>,
    ) -> CoreResult<Vec<EntryInfo>> {
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
                    open_as: None,
                })
                .collect();
            sort_entries(&mut out, sort.unwrap_or_default());
            Ok(out)
        })
    }

    /// Decrypts one file of the package into memory for the viewer.
    /// `open_as` overrides what the file looks like (remembered per file
    /// type on this device, since a package cannot be changed).
    pub fn open_viewer(&self, id: String, open_as: Option<PreviewKind>) -> CoreResult<FileViewer> {
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
        FileViewer::new(
            name,
            data,
            open_as.map(Into::into),
            &self.inner.viewers,
            move || inner.is_closed(),
        )
    }

    /// Saves edited text as the new contents of file `id`. The package is
    /// written again under the same lock and swapped in place.
    pub fn save_text(&self, id: String, text: String) -> CoreResult<()> {
        let index = parse_index(&id)?;
        let text = Zeroizing::new(text);
        self.with(|r| r.replace_file(index, text.as_bytes()))
    }

    /// Saves the picture open in `viewer` with `edits` applied over file
    /// `id`, rewriting the package in place.
    pub fn save_edited_image(
        &self,
        viewer: &FileViewer,
        id: String,
        edits: Vec<ImageEdit>,
    ) -> CoreResult<()> {
        let index = parse_index(&id)?;
        let data = viewer.encode_edited(edits)?;
        self.with(|r| r.replace_file(index, &data))
    }

    /// Writes one file of the package out, unencrypted, into `dest_dir`.
    /// Returns the path written (a free name, never overwriting).
    pub fn extract_file(&self, id: String, dest_dir: String) -> CoreResult<String> {
        let index = parse_index(&id)?;
        self.with(|r| {
            let item = r.items().get(index).cloned().ok_or_else(closed)?;
            if item.kind != Kind::File {
                return Err(CoreError::InvalidInput {
                    message: "only files can be extracted one at a time".into(),
                });
            }
            let name = item.path.last().cloned().unwrap_or_default();
            let dest = crate::vault::free_disk_name(Path::new(&dest_dir), &name);
            let part = dest.with_extension("atropos-part");
            let result = (|| -> CoreResult<()> {
                let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
                r.read_file(index, &mut out)?;
                out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
                std::fs::rename(&part, &dest)?;
                Ok(())
            })();
            if let Err(e) = result {
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
            Ok(dest.to_string_lossy().into_owned())
        })
    }

    /// Wipes the package key and every file opened from it.
    #[frb(sync)]
    pub fn close(&self) {
        self.inner.close();
    }
}
