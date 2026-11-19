//! App settings and the list of known vaults, stored as JSON in the app's
//! private config folder. Neither file holds anything secret.
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct AppSettings {
    /// Minutes in the background before vaults lock; 0 = as soon as the app
    /// leaves the screen (including screen off).
    pub auto_lock_minutes: u32,
    /// Type passwords on the Atropos keypad instead of the system keyboard.
    pub secure_keyboard: bool,
    pub check_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_lock_minutes: 0,
            secure_keyboard: true,
            check_updates: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct KnownVault {
    pub path: String,
    pub name: String,
    pub last_opened: i64,
}

const SETTINGS: &str = "settings.json";
const VAULTS: &str = "vaults.json";

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
    read(&config_dir, SETTINGS)
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
    v.retain(|k| k.path != path);
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    v.push(KnownVault {
        path,
        name,
        last_opened: crate::vault::now(),
    });
    write(&config_dir, VAULTS, &v)
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
        assert_eq!(list_vaults(dir)[0].name, "Work");
    }
}
