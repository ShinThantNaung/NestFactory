//! Settings and per-project data kept in the app data folder, never in the project, so
//! projects stay free of NestFactory files. Database passwords go to the OS keychain.

use std::path::{Path, PathBuf};

use anyhow::Result;
use nestfactory_core::db::DbConfig;
use nestfactory_core::model::PackageManager;
use nestfactory_core::project::read_meta;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use ts_rs::TS;

use crate::http::{ApiRequest, HistoryEntry};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    /// dark | light | system
    pub theme: String,
    pub editor_font_size: u32,
    pub type_check_on_save: bool,
    /// Format with the project's Prettier (or the built-in formatter) on save.
    pub format_on_save: bool,
    /// The terminal profile new terminals open with (`pwsh`, `cmd`, `gitbash`…); empty for the first found.
    pub terminal_profile: String,
    pub default_package_manager: PackageManager,
    pub default_location: String,
    pub recent_projects: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        Self {
            theme: "dark".into(),
            editor_font_size: 13,
            type_check_on_save: true,
            format_on_save: true,
            terminal_profile: String::new(),
            default_package_manager: PackageManager::Npm,
            default_location: if home.is_empty() { String::new() } else { format!("{home}{}dev", std::path::MAIN_SEPARATOR) },
            recent_projects: vec![],
        }
    }
}

/// A recent project as the landing screen lists it, read fresh from its package.json.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentProject {
    pub path: String,
    /// package.json `name`, else the folder name.
    pub name: String,
    pub nest_version: Option<String>,
    pub package_manager: PackageManager,
    /// The folder or its package.json is gone (moved, deleted, unplugged drive).
    pub missing: bool,
}

/// A saved connection. The password is in the OS keychain, not here.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DbProfile {
    pub config: DbConfig,
    pub has_password: bool,
    pub module_file: Option<String>,
    pub container: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectData {
    pub db_profiles: Vec<DbProfile>,
    pub saved_requests: Vec<ApiRequest>,
    pub history: Vec<HistoryEntry>,
}

pub struct Store {
    dir: PathBuf,
}

fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn project_key(root: &Path) -> String {
    // FNV-1a of the normalized path: stable across runs, no dependency.
    let s = root.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn settings(&self) -> Settings {
        read(&self.dir.join("settings.json"))
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        write(&self.dir.join("settings.json"), s)
    }

    pub fn add_recent(&self, root: &Path) -> Result<Settings> {
        let mut s = self.settings();
        let p = root.to_string_lossy().to_string();
        s.recent_projects.retain(|r| !r.eq_ignore_ascii_case(&p));
        s.recent_projects.insert(0, p);
        s.recent_projects.truncate(10);
        self.save_settings(&s)?;
        Ok(s)
    }

    /// Removes one path from the recent list, or all of them when `path` is `None`.
    pub fn remove_recent(&self, path: Option<&str>) -> Result<Settings> {
        let mut s = self.settings();
        match path {
            Some(p) => s.recent_projects.retain(|r| !r.eq_ignore_ascii_case(p)),
            None => s.recent_projects.clear(),
        }
        self.save_settings(&s)?;
        Ok(s)
    }

    pub fn recent_projects(&self) -> Vec<RecentProject> {
        self.settings()
            .recent_projects
            .into_iter()
            .map(|path| {
                let root = Path::new(&path);
                let missing = !root.join("package.json").is_file();
                let meta = read_meta(root);
                RecentProject {
                    nest_version: if missing { None } else { meta.major_version("@nestjs/core") },
                    name: meta.name,
                    package_manager: meta.package_manager,
                    missing,
                    path,
                }
            })
            .collect()
    }

    fn project_path(&self, root: &Path) -> PathBuf {
        self.dir.join("projects").join(format!("{}.json", project_key(root)))
    }

    pub fn project(&self, root: &Path) -> ProjectData {
        read(&self.project_path(root))
    }

    pub fn update_project<F: FnOnce(&mut ProjectData)>(&self, root: &Path, f: F) -> Result<ProjectData> {
        let mut d = self.project(root);
        f(&mut d);
        write(&self.project_path(root), &d)?;
        Ok(d)
    }
}

const KEYRING_SERVICE: &str = "NestFactory";

pub fn set_password(root: &Path, profile: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{}/{profile}", project_key(root)))?;
    if password.is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry.set_password(password)?;
    }
    Ok(())
}

pub fn get_password(root: &Path, profile: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{}/{profile}", project_key(root))).ok()?.get_password().ok()
}

pub fn delete_password(root: &Path, profile: &str) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &format!("{}/{profile}", project_key(root))) {
        let _ = e.delete_credential();
    }
}
