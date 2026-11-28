//! "Check vault": decrypts and verifies everything, and finds what a crash,
//! a broken sync or a bad disk left behind. "Repair" then tidies it up.
//!
//! Nothing is ever deleted that could still hold readable data: damaged
//! manifest segments are moved into `damaged/`, not removed, and files are
//! only taken off the list when the person asks for it.
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::blob::blob_path;
use super::manifest::{Id, Kind, Record};
use super::{Progress, Vault, MANIFEST_DIR, OBJECTS_DIR};
use crate::error::{CoreError, CoreResult};

/// Where repair puts manifest segments it could not read.
pub const DAMAGED_DIR: &str = "damaged";

/// One item with a problem, and its path for messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub id: Id,
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub files_checked: u64,
    pub bytes_checked: u64,
    /// Files whose contents failed to decrypt or did not match their hash.
    pub damaged: Vec<Problem>,
    /// Files whose blob is gone (deleted, or not synced to this device yet).
    pub missing: Vec<Problem>,
    /// Items whose folder no longer exists; repair moves them to the top.
    pub lost: Vec<Problem>,
    /// Blob files nothing refers to, usually from an interrupted import.
    pub orphans: Vec<PathBuf>,
    pub orphan_bytes: u64,
    /// Temporary files from interrupted writes.
    pub leftovers: Vec<PathBuf>,
    /// File-list segments that could not be read.
    pub damaged_segments: Vec<PathBuf>,
}

impl CheckReport {
    pub fn is_clean(&self) -> bool {
        self.damaged.is_empty()
            && self.missing.is_empty()
            && self.lost.is_empty()
            && self.orphans.is_empty()
            && self.leftovers.is_empty()
            && self.damaged_segments.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairOutcome {
    pub removed_entries: u64,
    pub moved_to_top: u64,
    pub deleted_orphans: u64,
    pub freed_bytes: u64,
    pub deleted_leftovers: u64,
    pub segments_set_aside: u64,
}

struct Discard;

impl Write for Discard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Another device may have written this file and not yet synced the file
/// list that mentions it, so only files this old count as left over.
pub const SETTLE: Duration = Duration::from_secs(24 * 3600);

fn settled(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|d| d >= age)
}

fn blob_id_of(path: &Path) -> Option<Id> {
    let name = path.file_name()?.to_str()?;
    let hex_id = name.strip_suffix(".blob")?;
    hex::decode(hex_id).ok()?.try_into().ok()
}

impl Vault {
    /// Path of an entry for messages, such as "Photos/2024/a.jpg".
    fn display_path(&self, id: &Id) -> String {
        let m = self.manifest();
        let mut parts = Vec::new();
        let mut cur = m.get(id);
        let mut guard = 0;
        while let Some(e) = cur {
            parts.push(e.name.clone());
            cur = e.parent.and_then(|p| m.get(&p));
            guard += 1;
            if guard > 256 {
                break;
            }
        }
        parts.reverse();
        parts.join("/")
    }

    /// Reads every file back (authenticating every chunk and checking its
    /// SHA-256) and looks for anything left over or missing.
    pub fn check(&mut self, progress: &mut impl FnMut(&Progress)) -> CoreResult<CheckReport> {
        let mut report = CheckReport {
            damaged_segments: self.manifest.damaged.clone(),
            ..CheckReport::default()
        };
        let objects = self.path.join(OBJECTS_DIR);
        let single = self.is_single();
        let files: Vec<(Id, u64, Option<Id>)> = self
            .manifest
            .entries
            .values()
            .filter(|e| e.kind == Kind::File)
            .map(|e| (e.id, e.size, e.data.as_ref().map(|d| d.blob)))
            .collect();
        let mut p = Progress {
            files_total: files.len() as u64,
            bytes_total: files.iter().map(|f| f.1).sum(),
            ..Progress::default()
        };
        progress(&p);
        let mut used: BTreeSet<Id> = BTreeSet::new();
        let mut missing_blobs: BTreeSet<Id> = BTreeSet::new();
        for (id, size, blob) in files {
            p.current = self.display_path(&id);
            if let Some(b) = blob {
                used.insert(b);
                let absent = !single && !blob_path(&objects, &b).exists();
                if missing_blobs.contains(&b) || absent {
                    missing_blobs.insert(b);
                    report.missing.push(Problem {
                        id,
                        path: p.current.clone(),
                    });
                } else {
                    match self.read_file(id, &mut Discard) {
                        Ok(_) => {
                            report.files_checked += 1;
                            report.bytes_checked += size;
                        }
                        Err(_) => report.damaged.push(Problem {
                            id,
                            path: p.current.clone(),
                        }),
                    }
                }
            } else {
                report.files_checked += 1;
            }
            p.files_done += 1;
            p.bytes_done += size;
            progress(&p);
        }
        // Items whose parent folder is gone.
        for e in self.manifest.entries.values() {
            if let Some(parent) = e.parent {
                let ok = self
                    .manifest
                    .get(&parent)
                    .is_some_and(|p| p.kind == Kind::Dir);
                if !ok {
                    report.lost.push(Problem {
                        id: e.id,
                        path: e.name.clone(),
                    });
                }
            }
        }
        let mut all = Vec::new();
        if !single {
            walk(&objects, &mut all);
            walk(&self.path.join(MANIFEST_DIR), &mut all);
        }
        for f in all {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "tmp" {
                if settled(&f, Duration::from_secs(3600)) {
                    report.leftovers.push(f);
                }
            } else if ext == "blob"
                && f.starts_with(&objects)
                && blob_id_of(&f).is_none_or(|id| !used.contains(&id))
                && settled(&f, SETTLE)
            {
                report.orphan_bytes += fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                report.orphans.push(f);
            }
        }
        for name in ["vault.tmp", "vault.hdr.tmp"] {
            let f = self.path.join(name);
            if !single && f.exists() {
                report.leftovers.push(f);
            }
        }
        report.leftovers.sort();
        report.orphans.sort();
        self.last_check = Some(report.clone());
        Ok(report)
    }

    /// Fixes what the last [Vault::check] found. Missing or damaged files
    /// are taken off the list only with `remove_broken`.
    pub fn repair(&mut self, remove_broken: bool) -> CoreResult<RepairOutcome> {
        let report = self
            .last_check
            .take()
            .ok_or_else(|| CoreError::InvalidInput {
                message: "check the vault first".into(),
            })?;
        let mut out = RepairOutcome::default();
        let objects = self.path.join(OBJECTS_DIR);

        out.segments_set_aside =
            self.manifest
                .set_aside_damaged(&self.path.join(DAMAGED_DIR))? as u64;

        let mut records = Vec::new();
        // Blobs of removed files; deleted below once nothing uses them.
        let mut freed_blobs = Vec::new();
        if remove_broken {
            for pr in report.damaged.iter().chain(&report.missing) {
                let id = &pr.id;
                if let Some(e) = self.manifest.get(id) {
                    freed_blobs.extend(e.data.as_ref().map(|d| d.blob));
                    records.push(Record::Remove { id: *id });
                    out.removed_entries += 1;
                }
            }
        }
        // Lost items go to the top level under a free name.
        for pr in &report.lost {
            let id = &pr.id;
            if let Some(mut e) = self.manifest.get(id).cloned() {
                e.parent = None;
                e.name = self.manifest.free_name(None, &e.name, Some(e.id));
                records.push(Record::Put { entry: e });
                out.moved_to_top += 1;
            }
        }
        self.manifest.commit(records)?;

        let mut doomed = report.orphans.clone();
        if !self.is_single() {
            doomed.extend(freed_blobs.iter().map(|b| blob_path(&objects, b)));
        }
        doomed.sort();
        doomed.dedup();
        for f in doomed {
            // Only ever delete blobs that are still unused right now.
            if blob_id_of(&f).is_some_and(|id| self.manifest.blob_in_use(&id)) {
                continue;
            }
            let len = fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            if fs::remove_file(&f).is_ok() {
                out.deleted_orphans += 1;
                out.freed_bytes += len;
            }
        }
        for f in &report.leftovers {
            if fs::remove_file(f).is_ok() {
                out.deleted_leftovers += 1;
            }
        }
        if let super::Store::Folder(blobs) = &mut self.store {
            blobs.forget_keys();
        }
        self.persist(&mut |_| {})?;
        Ok(out)
    }
}
