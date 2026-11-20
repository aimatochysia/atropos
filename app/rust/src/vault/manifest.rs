//! The encrypted file tree, stored as an append-only journal of segments in
//! `manifest/`. Each segment is one AES-256-GCM-sealed batch of records, so a
//! crash can lose at most the batch being written, never earlier ones.
//! Segment names are `<sequence>-<random>.seg`: two devices writing the same
//! sequence number produce two files instead of a sync conflict.
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crypto::aead::AeadKey;
use crate::crypto::random_array;
use crate::error::{CoreError, CoreResult};

pub type Id = [u8; 16];

/// Compact the journal into one snapshot once it has this many segments.
const COMPACT_AFTER: usize = 64;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

/// Where a file's bytes live: a range inside one blob.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct DataRef {
    #[serde(with = "hex_id")]
    pub blob: Id,
    pub offset: u64,
    pub len: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    #[serde(with = "hex_id")]
    pub id: Id,
    #[serde(with = "hex_id_opt", default)]
    pub parent: Option<Id>,
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
    #[serde(default)]
    pub executable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<DataRef>,
    /// SHA-256 of the plaintext, checked on export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// How the person chose to open this file ("text", "image", ...),
    /// overriding what Atropos detects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_as: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Record {
    Put {
        entry: Entry,
    },
    Remove {
        #[serde(with = "hex_id")]
        id: Id,
    },
}

#[derive(Serialize, Deserialize)]
struct Segment {
    /// A snapshot replaces everything in earlier segments.
    snapshot: bool,
    records: Vec<Record>,
}

mod hex_id {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(id: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(id))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(s).map_err(serde::de::Error::custom)?;
        v.try_into().map_err(|_| serde::de::Error::custom("bad id"))
    }
}

mod hex_id_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(id: &Option<[u8; 16]>, s: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => s.serialize_some(&hex::encode(id)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 16]>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        s.map(|s| {
            let v = hex::decode(s).map_err(serde::de::Error::custom)?;
            v.try_into().map_err(|_| serde::de::Error::custom("bad id"))
        })
        .transpose()
    }
}

/// The decrypted tree of an unlocked vault.
pub struct Manifest {
    dir: PathBuf,
    key: AeadKey,
    vault_id: Vec<u8>,
    next_seq: u64,
    segments: Vec<PathBuf>,
    /// Segments that failed to decrypt and were skipped (see [Manifest::load]).
    pub damaged: Vec<PathBuf>,
    pub entries: BTreeMap<Id, Entry>,
}

fn aad(vault_id: &[u8], seq: u64) -> Vec<u8> {
    let mut a = b"atropos/manifest/v1".to_vec();
    a.extend_from_slice(vault_id);
    a.extend_from_slice(&seq.to_be_bytes());
    a
}

fn parse_name(p: &Path) -> Option<(u64, String)> {
    let name = p.file_name()?.to_str()?;
    let stem = name.strip_suffix(".seg")?;
    let (seq, rand) = stem.split_once('-')?;
    Some((u64::from_str_radix(seq, 16).ok()?, rand.to_string()))
}

impl Manifest {
    pub fn load(dir: &Path, key: &[u8; 32], vault_id: &[u8]) -> CoreResult<Self> {
        fs::create_dir_all(dir)?;
        let mut files: Vec<(u64, String, PathBuf)> = Vec::new();
        for e in fs::read_dir(dir)? {
            let p = e?.path();
            if let Some((seq, rand)) = parse_name(&p) {
                files.push((seq, rand, p));
            }
        }
        files.sort();
        let key = AeadKey::new(key)?;
        let mut entries = BTreeMap::new();
        let mut segments = Vec::new();
        let mut next_seq = 0;
        let mut damaged = Vec::new();
        for (seq, _, path) in files {
            next_seq = next_seq.max(seq + 1);
            match open_segment(&key, vault_id, seq, &path) {
                Some(segment) => {
                    if segment.snapshot {
                        entries.clear();
                    }
                    apply(&mut entries, segment.records);
                    segments.push(path);
                }
                // One unreadable segment (a bad sector, a broken sync) must
                // not lock the person out of everything else; "Check vault"
                // reports it and can set it aside.
                None => damaged.push(path),
            }
        }
        if segments.is_empty() && !damaged.is_empty() {
            return Err(damaged_error());
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            key,
            vault_id: vault_id.to_vec(),
            next_seq,
            segments,
            damaged,
            entries,
        })
    }

    /// Writes a fresh snapshot (so skipped segments no longer matter) and
    /// moves the damaged segments into `aside`. Returns how many moved.
    pub fn set_aside_damaged(&mut self, aside: &Path) -> CoreResult<usize> {
        if self.damaged.is_empty() {
            return Ok(0);
        }
        self.compact()?;
        fs::create_dir_all(aside)?;
        let mut moved = 0;
        for p in std::mem::take(&mut self.damaged) {
            if let Some(name) = p.file_name() {
                if fs::rename(&p, aside.join(name)).is_ok() {
                    moved += 1;
                }
            }
        }
        Ok(moved)
    }

    /// Appends one batch durably, then applies it in memory.
    pub fn commit(&mut self, records: Vec<Record>) -> CoreResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let path = self.write_segment(false, &records)?;
        self.segments.push(path);
        apply(&mut self.entries, records);
        if self.segments.len() > COMPACT_AFTER {
            self.compact()?;
        }
        Ok(())
    }

    /// Rewrites the whole tree as one snapshot and removes older segments.
    pub fn compact(&mut self) -> CoreResult<()> {
        let records = self
            .entries
            .values()
            .map(|e| Record::Put { entry: e.clone() })
            .collect::<Vec<_>>();
        let path = self.write_segment(true, &records)?;
        for old in std::mem::take(&mut self.segments) {
            let _ = fs::remove_file(old);
        }
        self.segments.push(path);
        Ok(())
    }

    fn write_segment(&mut self, snapshot: bool, records: &[Record]) -> CoreResult<PathBuf> {
        let seq = self.next_seq;
        let mut plain =
            zeroize::Zeroizing::new(serde_json::to_vec(&SegmentRef { snapshot, records })?);
        // Pad with JSON whitespace so a segment's size reveals little.
        let padded = crate::crypto::padme(plain.len().max(256) as u64) as usize;
        plain.resize(padded, b' ');
        let nonce = random_array::<12>()?;
        let sealed = self.key.seal(&nonce, &aad(&self.vault_id, seq), &plain)?;
        let name = format!("{seq:016x}-{}.seg", hex::encode(random_array::<4>()?));
        let path = self.dir.join(name);
        write_atomic(&path, &[&nonce, &sealed])?;
        self.next_seq = seq + 1;
        Ok(path)
    }

    pub fn get(&self, id: &Id) -> Option<&Entry> {
        self.entries.get(id)
    }

    pub fn children(&self, parent: Option<Id>) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self
            .entries
            .values()
            .filter(|e| e.parent == parent)
            .collect();
        v.sort_by(|a, b| {
            (a.kind != Kind::Dir)
                .cmp(&(b.kind != Kind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        v
    }

    /// `id` and everything below it.
    pub fn subtree(&self, id: Id) -> Vec<Id> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            let cur = out[i];
            out.extend(
                self.entries
                    .values()
                    .filter(|e| e.parent == Some(cur))
                    .map(|e| e.id),
            );
            i += 1;
        }
        out
    }

    /// A name not yet used in `parent`: "a.txt", "a (2).txt", ...
    pub fn free_name(&self, parent: Option<Id>, wanted: &str, ignore: Option<Id>) -> String {
        let taken = |n: &str| {
            self.entries.values().any(|e| {
                e.parent == parent && Some(e.id) != ignore && e.name.eq_ignore_ascii_case(n)
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

    /// True if any entry still points into `blob`.
    pub fn blob_in_use(&self, blob: &Id) -> bool {
        self.entries
            .values()
            .any(|e| e.data.as_ref().is_some_and(|d| &d.blob == blob))
    }
}

#[derive(Serialize)]
struct SegmentRef<'a> {
    snapshot: bool,
    records: &'a [Record],
}

fn apply(entries: &mut BTreeMap<Id, Entry>, records: Vec<Record>) {
    for r in records {
        match r {
            Record::Put { entry } => {
                entries.insert(entry.id, entry);
            }
            Record::Remove { id } => {
                entries.remove(&id);
            }
        }
    }
}

fn open_segment(key: &AeadKey, vault_id: &[u8], seq: u64, path: &Path) -> Option<Segment> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 12 {
        return None;
    }
    let nonce: [u8; 12] = bytes[..12].try_into().ok()?;
    let plain = key.open(&nonce, &aad(vault_id, seq), &bytes[12..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

fn damaged_error() -> CoreError {
    CoreError::InvalidInput {
        message: "the vault's file list is damaged or was changed outside Atropos".into(),
    }
}

/// Writes `parts` to `path` via a temporary file, fsyncs, then renames, so
/// readers only ever see a complete file.
pub fn write_atomic(path: &Path, parts: &[&[u8]]) -> CoreResult<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        for p in parts {
            f.write_all(p)?;
        }
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(d) = fs::File::open(parent) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u8, parent: Option<u8>, name: &str) -> Entry {
        Entry {
            id: [id; 16],
            parent: parent.map(|p| [p; 16]),
            name: name.into(),
            kind: Kind::File,
            size: 1,
            modified: 0,
            executable: false,
            link_target: None,
            data: None,
            sha256: None,
            open_as: None,
        }
    }

    #[test]
    fn journal_survives_reload_and_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let key = [3u8; 32];
        let mut m = Manifest::load(dir.path(), &key, b"vault").unwrap();
        for i in 1..=70u8 {
            m.commit(vec![Record::Put {
                entry: entry(i, None, &format!("f{i}")),
            }])
            .unwrap();
        }
        m.commit(vec![Record::Remove { id: [5; 16] }]).unwrap();
        assert!(m.segments.len() < 10, "compacted");
        let again = Manifest::load(dir.path(), &key, b"vault").unwrap();
        assert_eq!(again.entries.len(), 69);
        assert!(again.get(&[5; 16]).is_none());
        assert_eq!(again.free_name(None, "f1", None), "f1 (2)");
        assert_eq!(again.free_name(None, "new", None), "new");
    }

    #[test]
    fn wrong_key_or_tampering_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Manifest::load(dir.path(), &[1; 32], b"v").unwrap();
        m.commit(vec![Record::Put {
            entry: entry(1, None, "a"),
        }])
        .unwrap();
        assert!(Manifest::load(dir.path(), &[2; 32], b"v").is_err());
        assert!(Manifest::load(dir.path(), &[1; 32], b"other").is_err());
        let seg = &m.segments[0];
        let mut bytes = fs::read(seg).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(seg, bytes).unwrap();
        assert!(Manifest::load(dir.path(), &[1; 32], b"v").is_err());
    }

    #[test]
    fn subtree_and_children() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Manifest::load(dir.path(), &[1; 32], b"v").unwrap();
        let mut d = entry(1, None, "dir");
        d.kind = Kind::Dir;
        m.commit(vec![
            Record::Put { entry: d },
            Record::Put {
                entry: entry(2, Some(1), "b"),
            },
            Record::Put {
                entry: entry(3, Some(1), "A"),
            },
            Record::Put {
                entry: entry(4, None, "top"),
            },
        ])
        .unwrap();
        assert_eq!(m.subtree([1; 16]).len(), 3);
        let names: Vec<_> = m
            .children(Some([1; 16]))
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(names, vec!["A", "b"]);
        assert_eq!(m.children(None)[0].name, "dir");
    }
}
