//! Vault upkeep: check and repair, and how hard the password is to guess.
use std::time::Instant;

use flutter_rust_bridge::frb;

use super::secure_input::SecretInput;
use super::vault::{ProgressInfo, VaultSession};
use crate::crypto::password::Argon2Cost;
use crate::error::{CoreError, CoreResult};
use crate::frb_generated::StreamSink;
use crate::vault::check::{CheckReport, RepairOutcome};
use crate::vault::Progress;

pub struct CheckReportInfo {
    pub files_checked: u64,
    pub bytes_checked: u64,
    /// Paths of files that failed to decrypt or did not match their hash.
    pub damaged: Vec<String>,
    /// Paths of files whose data is missing (deleted or not synced yet).
    pub missing: Vec<String>,
    /// Items whose folder is gone; repair moves them to the top level.
    pub lost: Vec<String>,
    pub orphans: u32,
    pub orphan_bytes: u64,
    pub leftovers: u32,
    pub damaged_segments: u32,
    pub clean: bool,
}

impl From<CheckReport> for CheckReportInfo {
    fn from(r: CheckReport) -> Self {
        let clean = r.is_clean();
        CheckReportInfo {
            files_checked: r.files_checked,
            bytes_checked: r.bytes_checked,
            damaged: r.damaged.into_iter().map(|p| p.path).collect(),
            missing: r.missing.into_iter().map(|p| p.path).collect(),
            lost: r.lost.into_iter().map(|p| p.path).collect(),
            orphans: r.orphans.len() as u32,
            orphan_bytes: r.orphan_bytes,
            leftovers: r.leftovers.len() as u32,
            damaged_segments: r.damaged_segments.len() as u32,
            clean,
        }
    }
}

pub enum CheckEvent {
    Progress(ProgressInfo),
    Done(CheckReportInfo),
}

pub struct RepairInfo {
    pub removed_entries: u32,
    pub moved_to_top: u32,
    pub deleted_orphans: u32,
    pub freed_bytes: u64,
    pub deleted_leftovers: u32,
    pub segments_set_aside: u32,
}

impl From<RepairOutcome> for RepairInfo {
    fn from(o: RepairOutcome) -> Self {
        RepairInfo {
            removed_entries: o.removed_entries as u32,
            moved_to_top: o.moved_to_top as u32,
            deleted_orphans: o.deleted_orphans as u32,
            freed_bytes: o.freed_bytes,
            deleted_leftovers: o.deleted_leftovers as u32,
            segments_set_aside: o.segments_set_aside as u32,
        }
    }
}

/// How much work (and memory) guessing one password takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordStrength {
    /// 64 MiB: for phones with little memory. Unlocks fastest.
    Light,
    /// 256 MiB, the default.
    Standard,
    /// 1 GiB: much slower to attack, and to unlock.
    Strong,
    /// Something else (set by another version or by hand).
    Custom,
}

fn cost_of(s: PasswordStrength) -> Option<Argon2Cost> {
    match s {
        PasswordStrength::Light => Some(Argon2Cost::MINIMUM),
        PasswordStrength::Standard => Some(Argon2Cost::DEFAULT),
        PasswordStrength::Strong => Some(Argon2Cost {
            memory_kib: 1024 * 1024,
            iterations: 4,
            parallelism: 1,
        }),
        PasswordStrength::Custom => None,
    }
}

fn strength_of(c: Argon2Cost) -> PasswordStrength {
    [
        PasswordStrength::Light,
        PasswordStrength::Standard,
        PasswordStrength::Strong,
    ]
    .into_iter()
    .find(|s| cost_of(*s) == Some(c))
    .unwrap_or(PasswordStrength::Custom)
}

impl VaultSession {
    /// Decrypts every file and looks for damage or leftovers. The report is
    /// kept for [VaultSession::repair].
    pub fn check(&self, sink: StreamSink<CheckEvent>) -> CoreResult<()> {
        let mut last = Instant::now() - std::time::Duration::from_secs(1);
        let report = self.with(|v| {
            v.check(&mut |p: &Progress| {
                if p.files_done == p.files_total || last.elapsed().as_millis() >= 100 {
                    last = Instant::now();
                    let _ = sink.add(CheckEvent::Progress(super::vault::progress_info(p)));
                }
            })
        })?;
        let _ = sink.add(CheckEvent::Done(report.into()));
        Ok(())
    }

    /// Fixes what the last check found. With `remove_broken`, damaged and
    /// missing files are taken off the list.
    pub fn repair(&self, remove_broken: bool) -> CoreResult<RepairInfo> {
        self.with(|v| v.repair(remove_broken)).map(Into::into)
    }

    /// Number of file-list pieces that could not be read when the vault
    /// opened (0 when all is well). Never blocks.
    #[frb(sync)]
    pub fn damaged_parts(&self) -> u32 {
        self.try_with(|v| v.manifest().damaged.len() as u32)
            .unwrap_or(0)
    }

    pub fn password_strength(&self) -> CoreResult<PasswordStrength> {
        self.with(|v| {
            v.password_cost()
                .map(strength_of)
                .ok_or(CoreError::InvalidInput {
                    message: "this vault has no password".into(),
                })
        })
    }

    /// Re-wraps the password slot at `strength`. Returns how long one
    /// unlock now takes on this device, in milliseconds.
    pub fn set_password_strength(
        &self,
        password: &SecretInput,
        strength: PasswordStrength,
    ) -> CoreResult<u32> {
        let cost = cost_of(strength).ok_or(CoreError::InvalidInput {
            message: "pick light, standard or strong".into(),
        })?;
        let pw = password.bytes();
        // Try the cost first: if this device cannot afford the memory, the
        // app fails here, before the vault depends on it.
        let took = crate::crypto::password::time_cost(cost)?;
        self.with(|v| v.set_password_cost(&pw, cost))?;
        Ok(took.as_millis().min(u32::MAX as u128) as u32)
    }
}
