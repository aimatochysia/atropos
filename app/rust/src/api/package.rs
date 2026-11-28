//! Browsing a package in place: its files are listed and viewed straight
//! from the encrypted `.atropos` file, without writing anything to disk.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use super::secure_input::SecretInput;
use super::vault::{
    outcome_info, parse_parent, reporter, sort_entries, wrong_secret, EntryInfo, EntryKind,
    OutcomeInfo, SortOrder, TaskEvent, VaultSession,
};
use super::viewer::{FileViewer, ImageEdit, PreviewKind, Viewers};
use crate::error::{CoreError, CoreResult};
use crate::frb_generated::StreamSink;
use crate::vault::manifest::Kind;
use crate::vault::package::{Destination, PackageKey, PackageReader};
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
    path: String,
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
    PackageSession {
        inner,
        name,
        path: path.to_string(),
    }
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

/// An item's id is its path inside the package, names joined by `/`
/// (names never contain one).
fn parse_path(id: &str) -> Vec<String> {
    id.split('/').map(str::to_string).collect()
}

fn path_id(path: &[String]) -> String {
    path.join("/")
}

fn parse_parent_path(parent: Option<String>) -> Vec<String> {
    parent.map(|p| parse_path(&p)).unwrap_or_default()
}

fn not_found() -> CoreError {
    CoreError::InvalidInput {
        message: "that item is not in the package".into(),
    }
}

impl PackageSession {
    fn with<T>(&self, f: impl FnOnce(&mut PackageReader) -> CoreResult<T>) -> CoreResult<T> {
        let mut g = self.inner.reader.lock().unwrap_or_else(|e| e.into_inner());
        f(g.as_mut().ok_or_else(closed)?)
    }

    /// Runs `f` with the reader and the index of item `id`.
    fn with_item<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut PackageReader, usize) -> CoreResult<T>,
    ) -> CoreResult<T> {
        let path = parse_path(id);
        self.with(|r| {
            let i = r.find(&path).ok_or_else(not_found)?;
            f(r, i)
        })
    }

    /// Where the package file is.
    #[frb(sync, getter)]
    pub fn path(&self) -> String {
        self.path.clone()
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
        let base = parse_parent_path(parent);
        self.with(|r| {
            let items = r.items();
            if !base.is_empty() && !items.iter().any(|i| i.path == base) {
                return Err(not_found());
            }
            let base = base.as_slice();
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
                .map(|(_, it)| EntryInfo {
                    id: path_id(&it.path),
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
        let (name, data) = self.with_item(&id, |r, index| {
            let item = r.items()[index].clone();
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
        let text = Zeroizing::new(text);
        self.with_item(&id, |r, index| r.replace_file(index, text.as_bytes()))
    }

    /// Saves the picture open in `viewer` with `edits` applied over file
    /// `id`, rewriting the package in place.
    pub fn save_edited_image(
        &self,
        viewer: &FileViewer,
        id: String,
        edits: Vec<ImageEdit>,
    ) -> CoreResult<()> {
        let data = viewer.encode_edited(edits)?;
        self.with_item(&id, |r, index| r.replace_file(index, &data))
    }

    /// Saves the picture open in `viewer` with `edits` applied as a new
    /// file next to `id`. Returns the new file's id.
    pub fn save_edited_image_copy(
        &self,
        viewer: &FileViewer,
        id: String,
        edits: Vec<ImageEdit>,
    ) -> CoreResult<String> {
        let data = Zeroizing::new(viewer.encode_edited(edits)?);
        let path = parse_path(&id);
        let (parent, name) = path
            .split_last()
            .map(|(n, p)| (p.to_vec(), n.clone()))
            .ok_or_else(not_found)?;
        let copy = super::viewer::copy_name(&name);
        self.with(|r| r.create_file(&parent, &copy, &data))
            .map(|p| path_id(&p))
    }

    /// Creates an empty folder in `parent` (None = top level). Returns its
    /// id. Like every change, the package is written again and swapped in.
    pub fn create_folder(&self, parent: Option<String>, name: String) -> CoreResult<String> {
        let parent = parse_parent_path(parent);
        self.with(|r| r.create_folder(&parent, &name))
            .map(|p| path_id(&p))
    }

    /// Creates a text file in `parent`. Returns its id.
    pub fn create_file(
        &self,
        parent: Option<String>,
        name: String,
        text: String,
    ) -> CoreResult<String> {
        let parent = parse_parent_path(parent);
        let text = Zeroizing::new(text);
        self.with(|r| r.create_file(&parent, &name, text.as_bytes()))
            .map(|p| path_id(&p))
    }

    /// Adds files and folders from disk into `parent`.
    pub fn add_files(
        &self,
        sources: Vec<String>,
        parent: Option<String>,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let parent = parse_parent_path(parent);
        let sources: Vec<std::path::PathBuf> = sources.into_iter().map(Into::into).collect();
        let o = self.with(|r| r.add_from_disk(&parent, &sources, &mut reporter(&sink)))?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    /// Renames an item. Returns its new id (a folder's items move with it).
    pub fn rename(&self, id: String, name: String) -> CoreResult<String> {
        let path = parse_path(&id);
        self.with(|r| r.rename(&path, &name)).map(|p| path_id(&p))
    }

    pub fn move_to(&self, ids: Vec<String>, parent: Option<String>) -> CoreResult<()> {
        let paths: Vec<Vec<String>> = ids.iter().map(|i| parse_path(i)).collect();
        let parent = parse_parent_path(parent);
        self.with(|r| r.move_to(&paths, &parent))
    }

    pub fn copy_to(&self, ids: Vec<String>, parent: Option<String>) -> CoreResult<()> {
        let paths: Vec<Vec<String>> = ids.iter().map(|i| parse_path(i)).collect();
        let parent = parse_parent_path(parent);
        self.with(|r| r.copy_to(&paths, &parent))
    }

    pub fn delete(&self, ids: Vec<String>) -> CoreResult<()> {
        let paths: Vec<Vec<String>> = ids.iter().map(|i| parse_path(i)).collect();
        self.with(|r| r.delete(&paths))
    }

    /// Files, folders and bytes inside folder `id` (None = everything).
    pub fn totals(&self, id: Option<String>) -> CoreResult<OutcomeInfo> {
        let base = parse_parent_path(id);
        self.with(|r| {
            let mut o = OutcomeInfo {
                files: 0,
                folders: 0,
                bytes: 0,
                skipped: vec![],
                originals_deleted: false,
            };
            for it in r.items() {
                if it.path.len() <= base.len() || !it.path.starts_with(&base) {
                    continue;
                }
                match it.kind {
                    Kind::Dir => o.folders += 1,
                    _ => {
                        o.files += 1;
                        o.bytes += it.size;
                    }
                }
            }
            Ok(o)
        })
    }

    /// Copies everything into `vault` (inside `parent`), for turning a
    /// single-file vault into a Vault LFS. The package is left as it is.
    pub fn copy_into_vault(
        &self,
        vault: &VaultSession,
        parent: Option<String>,
        sink: StreamSink<TaskEvent>,
    ) -> CoreResult<()> {
        let parent = parse_parent(parent)?;
        let o = self.with(|r| {
            vault.with(|v| {
                r.extract(
                    Destination::Vault { vault: v, parent },
                    &mut reporter(&sink),
                )
            })
        })?;
        let _ = sink.add(TaskEvent::Done(outcome_info(o)));
        Ok(())
    }

    /// Writes one file of the package out, unencrypted, into `dest_dir`.
    /// Returns the path written (a free name, never overwriting).
    pub fn extract_file(&self, id: String, dest_dir: String) -> CoreResult<String> {
        self.with_item(&id, |r, index| {
            let item = r.items()[index].clone();
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
