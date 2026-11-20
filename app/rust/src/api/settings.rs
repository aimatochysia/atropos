//! App settings and the list of known vaults, stored as JSON in the app's
//! private config folder. Neither file holds anything secret.
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::vault::SortOrder;
use super::viewer::PreviewKind;
use crate::error::CoreResult;

/// How often Atropos looks for a new version.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheck {
    Off,
    #[default]
    EveryStart,
    Daily,
    Weekly,
}

/// "Always open files ending in .pub as text", for packages (which can't
/// store a choice per file).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OpenAsRule {
    /// Lowercase, without the dot.
    pub extension: String,
    pub kind: PreviewKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct AppSettings {
    /// Minutes in the background before vaults lock; 0 = as soon as the app
    /// leaves the screen (including screen off).
    pub auto_lock_minutes: u32,
    /// Type passwords on the Atropos keypad instead of the system keyboard.
    pub secure_keyboard: bool,
    pub update_check: UpdateCheck,
    /// Unix seconds of the last update check (for Daily and Weekly).
    pub last_update_check: i64,
    /// A version the person chose to skip; its popup is not shown again.
    pub skipped_version: String,
    /// The last version whose release notes popup was shown.
    pub notes_shown_for: String,
    /// List recently opened packages on Home (by name only).
    pub remember_packages: bool,
    /// Sort order inside packages.
    pub package_sort: SortOrder,
    pub open_as_rules: Vec<OpenAsRule>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_lock_minutes: 0,
            secure_keyboard: true,
            update_check: UpdateCheck::EveryStart,
            last_update_check: 0,
            skipped_version: String::new(),
            notes_shown_for: String::new(),
            remember_packages: true,
            package_sort: SortOrder::default(),
            open_as_rules: Vec::new(),
        }
    }
}

/// Whether an automatic update check is due now.
#[flutter_rust_bridge::frb(sync)]
pub fn update_check_due(settings: &AppSettings, now: i64) -> bool {
    let since = now - settings.last_update_check;
    match settings.update_check {
        UpdateCheck::Off => false,
        UpdateCheck::EveryStart => true,
        UpdateCheck::Daily => since >= 24 * 3600,
        UpdateCheck::Weekly => since >= 7 * 24 * 3600,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KnownVault {
    pub path: String,
    pub name: String,
    pub last_opened: i64,
    /// How this vault's lists are sorted.
    #[serde(default)]
    pub sort: SortOrder,
}

/// A recently opened package. Only `name` is shown; `path` is kept to open
/// it again.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KnownPackage {
    pub path: String,
    pub name: String,
    pub last_opened: i64,
}

const SETTINGS: &str = "settings.json";
const VAULTS: &str = "vaults.json";
const PACKAGES: &str = "packages.json";
const MAX_PACKAGES: usize = 20;

fn read<T: for<'de> Deserialize<'de> + Default>(dir: &str, name: &str) -> T {
    fs::read(Path::new(dir).join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write<T: Serialize>(dir: &str, name: &str, value: &T) -> CoreResult<()> {
    fs::create_dir_all(dir)?;
    crate::vault::manifest::write_atomic(
        &Path::new(dir).join(name),
        &[&serde_json::to_vec_pretty(value)?],
    )
}

pub fn load_settings(config_dir: String) -> AppSettings {
    let raw: serde_json::Value = read(&config_dir, SETTINGS);
    let mut s: AppSettings = serde_json::from_value(raw.clone()).unwrap_or_default();
    // Before v0.7 this was an on/off switch.
    if raw.get("update_check").is_none() && raw.get("check_updates") == Some(&false.into()) {
        s.update_check = UpdateCheck::Off;
    }
    s
}

pub fn save_settings(config_dir: String, settings: AppSettings) -> CoreResult<()> {
    write(&config_dir, SETTINGS, &settings)
}

/// Known vaults, most recently opened first. Vaults whose folder is gone are
/// still listed so a missing SD card does not forget them.
pub fn list_vaults(config_dir: String) -> Vec<KnownVault> {
    let mut v: Vec<KnownVault> = read(&config_dir, VAULTS);
    v.sort_by_key(|k| std::cmp::Reverse(k.last_opened));
    v
}

pub fn remember_vault(config_dir: String, path: String) -> CoreResult<()> {
    let mut v: Vec<KnownVault> = read(&config_dir, VAULTS);
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let sort = v
        .iter()
        .find(|k| k.path == path)
        .map(|k| k.sort)
        .unwrap_or_default();
    v.retain(|k| k.path != path);
    v.push(KnownVault {
        path,
        name,
        last_opened: crate::vault::now(),
        sort,
    });
    write(&config_dir, VAULTS, &v)
}

/// Remembers how a vault's lists are sorted.
pub fn set_vault_sort(config_dir: String, path: String, sort: SortOrder) -> CoreResult<()> {
    let mut v: Vec<KnownVault> = read(&config_dir, VAULTS);
    match v.iter_mut().find(|k| k.path == path) {
        Some(k) => k.sort = sort,
        None => return Ok(()),
    }
    write(&config_dir, VAULTS, &v)
}

/// Recently opened packages, newest first.
pub fn list_packages(config_dir: String) -> Vec<KnownPackage> {
    let mut v: Vec<KnownPackage> = read(&config_dir, PACKAGES);
    v.sort_by_key(|k| std::cmp::Reverse(k.last_opened));
    v
}

pub fn remember_package(config_dir: String, path: String) -> CoreResult<()> {
    let mut v = list_packages(config_dir.clone());
    v.retain(|k| k.path != path);
    let name = Path::new(&path)
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    v.insert(
        0,
        KnownPackage {
            path,
            name,
            last_opened: crate::vault::now(),
        },
    );
    v.truncate(MAX_PACKAGES);
    write(&config_dir, PACKAGES, &v)
}

pub fn forget_package(config_dir: String, path: String) -> CoreResult<()> {
    let mut v: Vec<KnownPackage> = read(&config_dir, PACKAGES);
    v.retain(|k| k.path != path);
    write(&config_dir, PACKAGES, &v)
}

/// Forgets every remembered package (when the list is switched off).
pub fn forget_all_packages(config_dir: String) -> CoreResult<()> {
    write(&config_dir, PACKAGES, &Vec::<KnownPackage>::new())
}

/// A random id for this installation, made on first use. Biometric key
/// slots in vault headers name the device they belong to with it.
pub fn device_id(config_dir: String) -> CoreResult<String> {
    let path = Path::new(&config_dir).join("device-id");
    if let Ok(s) = fs::read_to_string(&path) {
        let s = s.trim();
        if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(s.to_string());
        }
    }
    fs::create_dir_all(&config_dir)?;
    let id = hex::encode(crate::crypto::random_array::<16>()?);
    crate::vault::manifest::write_atomic(&path, &[id.as_bytes()])?;
    Ok(id)
}

/// Removes a vault from the list; its files are not touched.
pub fn forget_vault(config_dir: String, path: String) -> CoreResult<()> {
    let mut v: Vec<KnownVault> = read(&config_dir, VAULTS);
    v.retain(|k| k.path != path);
    write(&config_dir, VAULTS, &v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_vault_list() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_string_lossy().to_string();
        assert_eq!(load_settings(dir.clone()), AppSettings::default());
        let s = AppSettings {
            auto_lock_minutes: 5,
            ..Default::default()
        };
        save_settings(dir.clone(), s.clone()).unwrap();
        assert_eq!(load_settings(dir.clone()), s);
        remember_vault(dir.clone(), "/a/Work".into()).unwrap();
        remember_vault(dir.clone(), "/b/Home".into()).unwrap();
        remember_vault(dir.clone(), "/a/Work".into()).unwrap();
        let v = list_vaults(dir.clone());
        assert_eq!(v.len(), 2);
        forget_vault(dir.clone(), "/b/Home".into()).unwrap();
        assert_eq!(list_vaults(dir.clone())[0].name, "Work");
        let sort = SortOrder {
            by: super::super::vault::SortBy::Modified,
            descending: true,
        };
        set_vault_sort(dir.clone(), "/a/Work".into(), sort).unwrap();
        remember_vault(dir.clone(), "/a/Work".into()).unwrap();
        assert_eq!(list_vaults(dir.clone())[0].sort, sort);

        remember_package(dir.clone(), "/x/trip.atropos".into()).unwrap();
        remember_package(dir.clone(), "/x/tax.atropos".into()).unwrap();
        let p = list_packages(dir.clone());
        assert_eq!(p[0].name, "tax");
        assert_eq!(p.len(), 2);
        forget_package(dir.clone(), "/x/tax.atropos".into()).unwrap();
        assert_eq!(list_packages(dir.clone())[0].name, "trip");
        forget_all_packages(dir.clone()).unwrap();
        assert!(list_packages(dir).is_empty());
    }

    #[test]
    fn old_update_switch_carries_over() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_string_lossy().to_string();
        fs::write(
            d.path().join(SETTINGS),
            r#"{"auto_lock_minutes":2,"secure_keyboard":true,"check_updates":false}"#,
        )
        .unwrap();
        let s = load_settings(dir);
        assert_eq!(s.update_check, UpdateCheck::Off);
        assert_eq!(s.auto_lock_minutes, 2);
        let mut on = AppSettings::default();
        assert!(update_check_due(&on, 100));
        on.update_check = UpdateCheck::Daily;
        on.last_update_check = 100;
        assert!(!update_check_due(&on, 100 + 3600));
        assert!(update_check_due(&on, 100 + 24 * 3600));
    }
}
