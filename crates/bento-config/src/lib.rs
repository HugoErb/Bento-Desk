//! Persistence of Bento Desk.
//!
//! Files live in `%APPDATA%\BentoDesk\` (or next to the executable in
//! portable mode, when a `portable.txt` file is present):
//!
//! - `config.json`: preferences ([`AppConfig`]);
//! - `layouts.json`: fences of every display configuration ([`DesktopState`]);
//! - `snapshots\*.json`: named snapshots ([`Snapshot`]).
//!
//! Every file is an envelope `{ "schema_version": N, "data": ... }`. Loading
//! runs the migrations from the stored version to [`SCHEMA_VERSION`]. Writes
//! are atomic: the content goes to a temporary file that replaces the old
//! one. A corrupt file is kept aside (`*.corrupt-<time>`) and defaults load.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bento_core::model::{DesktopState, Snapshot};
use bento_core::settings::AppConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of the on-disk schema written by this build.
pub const SCHEMA_VERSION: u32 = 1;

const APP_DIR: &str = "BentoDesk";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no data directory: {0}")]
    NoDataDir(&'static str),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("file {path} uses schema {found}, newer than supported {supported}")]
    TooNew {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("invalid snapshot name")]
    BadName,
}

pub type Result<T> = std::result::Result<T, ConfigError>;

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> ConfigError + '_ {
    move |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Resolved locations of every file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub portable: bool,
}

impl Paths {
    /// Standard locations, or portable ones when `portable.txt` sits next
    /// to the executable.
    pub fn resolve() -> Result<Paths> {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
            && dir.join("portable.txt").is_file()
        {
            let data = dir.join("data");
            return Ok(Paths {
                log_dir: data.join("logs"),
                data_dir: data,
                portable: true,
            });
        }
        let roaming = std::env::var_os("APPDATA").ok_or(ConfigError::NoDataDir("APPDATA"))?;
        let local =
            std::env::var_os("LOCALAPPDATA").ok_or(ConfigError::NoDataDir("LOCALAPPDATA"))?;
        Ok(Paths {
            data_dir: PathBuf::from(roaming).join(APP_DIR),
            log_dir: PathBuf::from(local).join(APP_DIR).join("logs"),
            portable: false,
        })
    }

    /// Paths rooted in an arbitrary directory (tests, tools).
    pub fn in_dir(dir: &Path) -> Paths {
        Paths {
            data_dir: dir.to_owned(),
            log_dir: dir.join("logs"),
            portable: true,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data_dir.join("layouts.json")
    }

    pub fn snapshot_dir(&self) -> PathBuf {
        self.data_dir.join("snapshots")
    }

    /// File where the daemon publishes the current desktop items, read by
    /// the settings window to test rules.
    pub fn items_file(&self) -> PathBuf {
        self.data_dir.join("desktop-items.json")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir).map_err(io_err(&self.data_dir))?;
        fs::create_dir_all(&self.log_dir).map_err(io_err(&self.log_dir))?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    schema_version: u32,
    data: T,
}

/// Upgrades raw JSON from `from` to [`SCHEMA_VERSION`]. Each step is a pure
/// function on `serde_json::Value`, tested below.
fn migrate(kind: FileKind, from: u32, mut data: Value) -> Value {
    let mut v = from;
    while v < SCHEMA_VERSION {
        data = match (kind, v) {
            // Version 0 was the unversioned format: the data itself without
            // envelope. Nothing to change in the content.
            (_, 0) => data,
            _ => data,
        };
        v += 1;
    }
    data
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileKind {
    Config,
    State,
    Snapshot,
}

/// Reads an enveloped file. Missing file = `Ok(None)`.
fn read_versioned<T: DeserializeOwned>(path: &Path, kind: FileKind) -> Result<Option<T>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path)(e)),
    };
    let json_err = |source| ConfigError::Json {
        path: path.to_owned(),
        source,
    };
    let raw: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(json_err)?;
    let (version, data) = match raw {
        Value::Object(mut map)
            if map.contains_key("schema_version") && map.contains_key("data") =>
        {
            let version = map
                .get("schema_version")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0);
            (version, map.remove("data").unwrap_or(Value::Null))
        }
        other => (0, other),
    };
    if version > SCHEMA_VERSION {
        return Err(ConfigError::TooNew {
            path: path.to_owned(),
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    let data = migrate(kind, version, data);
    serde_json::from_value(data).map(Some).map_err(json_err)
}

/// Writes atomically: temporary file in the same directory, then rename
/// (which replaces the destination on Windows).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(io_err(&tmp))?;
        f.write_all(bytes).map_err(io_err(&tmp))?;
        f.sync_all().map_err(io_err(&tmp))?;
    }
    fs::rename(&tmp, path).map_err(io_err(path))
}

fn write_versioned<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    let env = Envelope {
        schema_version: SCHEMA_VERSION,
        data,
    };
    let bytes = serde_json::to_vec_pretty(&env).map_err(|source| ConfigError::Json {
        path: path.to_owned(),
        source,
    })?;
    write_atomic(path, &bytes)
}

/// Moves a corrupt file aside so that the user can recover it by hand.
fn quarantine(path: &Path) {
    let aside = path.with_extension(format!("corrupt-{}", bento_core::now_secs()));
    match fs::rename(path, &aside) {
        Ok(()) => tracing::warn!("corrupt file moved to {}", aside.display()),
        Err(e) => tracing::error!("cannot move corrupt file {}: {e}", path.display()),
    }
}

/// Loads a file, falling back to defaults (and quarantining it) on error.
fn load_or_default<T: DeserializeOwned + Default>(path: &Path, kind: FileKind) -> T {
    match read_versioned(path, kind) {
        Ok(Some(v)) => v,
        Ok(None) => T::default(),
        Err(ConfigError::TooNew { .. }) => {
            tracing::error!(
                "{} was written by a newer version, using defaults",
                path.display()
            );
            T::default()
        }
        Err(e) => {
            tracing::error!("{e}");
            quarantine(path);
            T::default()
        }
    }
}

pub fn load_config(paths: &Paths) -> AppConfig {
    load_or_default(&paths.config_file(), FileKind::Config)
}

pub fn save_config(paths: &Paths, cfg: &AppConfig) -> Result<()> {
    write_versioned(&paths.config_file(), cfg)
}

pub fn load_state(paths: &Paths) -> DesktopState {
    let mut s: DesktopState = load_or_default(&paths.state_file(), FileKind::State);
    s.repair_ids();
    for l in &mut s.layouts {
        l.normalize();
    }
    s
}

pub fn save_state(paths: &Paths, state: &DesktopState) -> Result<()> {
    write_versioned(&paths.state_file(), state)
}

/// File name for a snapshot: keeps letters, digits, spaces, dashes.
fn snapshot_file(paths: &Paths, name: &str) -> Result<PathBuf> {
    let clean: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_owned();
    if clean.is_empty() {
        return Err(ConfigError::BadName);
    }
    Ok(paths.snapshot_dir().join(format!("{clean}.json")))
}

pub fn save_snapshot(paths: &Paths, snap: &Snapshot) -> Result<()> {
    write_versioned(&snapshot_file(paths, &snap.name)?, snap)
}

pub fn load_snapshot(paths: &Paths, name: &str) -> Result<Option<Snapshot>> {
    read_versioned(&snapshot_file(paths, name)?, FileKind::Snapshot)
}

pub fn delete_snapshot(paths: &Paths, name: &str) -> Result<()> {
    let p = snapshot_file(paths, name)?;
    match fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(&p)(e)),
    }
}

/// Snapshots sorted from newest to oldest. Unreadable files are skipped.
pub fn list_snapshots(paths: &Paths) -> Vec<Snapshot> {
    let Ok(entries) = fs::read_dir(paths.snapshot_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<Snapshot> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(
            |p| match read_versioned::<Snapshot>(&p, FileKind::Snapshot) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("{e}");
                    None
                }
            },
        )
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Full export: configuration, layouts and snapshots in one JSON file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportBundle {
    pub config: AppConfig,
    pub state: DesktopState,
    pub snapshots: Vec<Snapshot>,
}

pub fn export_all(paths: &Paths, target: &Path) -> Result<()> {
    let bundle = ExportBundle {
        config: load_config(paths),
        state: load_state(paths),
        snapshots: list_snapshots(paths),
    };
    write_versioned(target, &bundle)
}

/// Reads an export file without applying it.
pub fn read_export(source: &Path) -> Result<ExportBundle> {
    read_versioned(source, FileKind::Config)?.ok_or_else(|| ConfigError::Io {
        path: source.to_owned(),
        source: std::io::Error::from(std::io::ErrorKind::NotFound),
    })
}

/// Replaces configuration, layouts and snapshots with an export.
pub fn import_all(paths: &Paths, bundle: &ExportBundle) -> Result<()> {
    save_config(paths, &bundle.config)?;
    save_state(paths, &bundle.state)?;
    for s in &bundle.snapshots {
        save_snapshot(paths, s)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bento_core::model::{Fence, FenceId, Layout, Page};

    fn temp_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("bento-config-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_state() -> DesktopState {
        let mut s = DesktopState::default();
        s.layouts.push(Layout {
            key: "k".into(),
            pages: vec![Page {
                fences: vec![Fence {
                    id: FenceId(7),
                    ..Fence::default()
                }],
                ..Page::default()
            }],
            ..Layout::default()
        });
        s
    }

    #[test]
    fn config_and_state_round_trip() {
        let dir = temp_dir("rt");
        let paths = Paths::in_dir(&dir);
        let mut cfg = AppConfig::default();
        cfg.quick_hide.idle_seconds = 12;
        save_config(&paths, &cfg).unwrap();
        assert_eq!(load_config(&paths), cfg);
        save_state(&paths, &sample_state()).unwrap();
        let s = load_state(&paths);
        assert_eq!(s.layouts[0].pages[0].fences[0].id, FenceId(7));
        assert!(s.next_id > 7);
        assert_eq!(s.layouts[0].pages[0].fences[0].tabs.len(), 1);
    }

    #[test]
    fn missing_files_give_defaults() {
        let dir = temp_dir("missing");
        let paths = Paths::in_dir(&dir);
        assert_eq!(load_config(&paths), AppConfig::default());
        assert!(load_state(&paths).layouts.is_empty());
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let dir = temp_dir("corrupt");
        let paths = Paths::in_dir(&dir);
        fs::write(paths.config_file(), "{ not json").unwrap();
        assert_eq!(load_config(&paths), AppConfig::default());
        assert!(!paths.config_file().exists());
        let aside = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt-"));
        assert!(aside);
    }

    #[test]
    fn unversioned_file_migrates() {
        let dir = temp_dir("v0");
        let paths = Paths::in_dir(&dir);
        fs::write(paths.config_file(), r#"{"quick_hide":{"idle_seconds":5}}"#).unwrap();
        assert_eq!(load_config(&paths).quick_hide.idle_seconds, 5);
    }

    #[test]
    fn newer_schema_is_refused() {
        let dir = temp_dir("new");
        let p = dir.join("x.json");
        fs::write(&p, r#"{"schema_version":999,"data":{}}"#).unwrap();
        let r: Result<Option<AppConfig>> = read_versioned(&p, FileKind::Config);
        assert!(matches!(r, Err(ConfigError::TooNew { .. })));
    }

    #[test]
    fn snapshots_and_export() {
        let dir = temp_dir("snap");
        let paths = Paths::in_dir(&dir);
        let snap = |name: &str, created| Snapshot {
            name: name.into(),
            created,
            layout: Layout::default(),
        };
        save_snapshot(&paths, &snap("Travail", 10)).unwrap();
        save_snapshot(&paths, &snap("Jeux 2", 20)).unwrap();
        assert!(save_snapshot(&paths, &snap("../..", 1)).is_err());
        let list = list_snapshots(&paths);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Jeux 2");
        assert!(load_snapshot(&paths, "Travail").unwrap().is_some());
        delete_snapshot(&paths, "Travail").unwrap();
        assert_eq!(list_snapshots(&paths).len(), 1);

        save_state(&paths, &sample_state()).unwrap();
        let export = dir.join("export.json");
        export_all(&paths, &export).unwrap();
        let bundle = read_export(&export).unwrap();
        assert_eq!(bundle.snapshots.len(), 1);
        let other = Paths::in_dir(&temp_dir("import"));
        import_all(&other, &bundle).unwrap();
        assert_eq!(load_state(&other).layouts.len(), 1);
    }
}
