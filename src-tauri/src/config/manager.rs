use crate::config::normalize::normalize;
use crate::config::schema::AppConfig;
use crate::error::{AppError, Result};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct ConfigManager {
    path: PathBuf,
    inner: RwLock<AppConfig>,
    notify: broadcast::Sender<AppConfig>,
}

impl ConfigManager {
    pub fn load(path: PathBuf) -> Result<Arc<Self>> {
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<AppConfig>(&text) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(?e, "config file unreadable, backing up and using defaults");
                    let backup = path.with_extension(format!(
                        "toml.corrupt-{}",
                        chrono::Local::now().format("%Y%m%d-%H%M%S")
                    ));
                    let _ = std::fs::rename(&path, &backup);
                    AppConfig::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("config file not found, using defaults");
                AppConfig::default()
            }
            Err(e) => return Err(AppError::Io(e)),
        };

        let dirty = normalize(&mut cfg);
        let (tx, _rx) = broadcast::channel(16);
        let mgr = Arc::new(Self {
            path: path.clone(),
            inner: RwLock::new(cfg.clone()),
            notify: tx,
        });

        // Persist normalized config (or initial defaults) on disk.
        if dirty || !path.exists() {
            mgr.persist(&cfg)?;
        }

        Ok(mgr)
    }

    pub fn snapshot(&self) -> AppConfig {
        self.inner.read().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppConfig> {
        self.notify.subscribe()
    }

    /// Apply mutation, normalize, persist, broadcast. Returns the new config.
    pub fn update<F>(&self, f: F) -> Result<AppConfig>
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut new_cfg = self.snapshot();
        f(&mut new_cfg);
        normalize(&mut new_cfg);
        self.persist(&new_cfg)?;
        *self.inner.write() = new_cfg.clone();
        let _ = self.notify.send(new_cfg.clone());
        Ok(new_cfg)
    }

    /// Replace entire config with a patch object that came from frontend.
    /// The patch is a serde_json::Value; missing fields keep current values.
    pub fn apply_patch(&self, patch: serde_json::Value) -> Result<AppConfig> {
        let current = self.snapshot();
        let mut current_value = serde_json::to_value(&current)
            .map_err(|e| AppError::Config(format!("serialize current: {e}")))?;
        merge_json(&mut current_value, patch);
        let mut new_cfg: AppConfig = serde_json::from_value(current_value)
            .map_err(|e| AppError::Config(format!("apply patch: {e}")))?;
        normalize(&mut new_cfg);
        self.persist(&new_cfg)?;
        *self.inner.write() = new_cfg.clone();
        let _ = self.notify.send(new_cfg.clone());
        Ok(new_cfg)
    }

    pub fn reset(&self) -> Result<AppConfig> {
        let new = AppConfig::default();
        self.persist(&new)?;
        *self.inner.write() = new.clone();
        let _ = self.notify.send(new.clone());
        Ok(new)
    }

    fn persist(&self, cfg: &AppConfig) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::Config("config path has no parent".into()))?;
        std::fs::create_dir_all(parent)?;
        write_atomic(&self.path, cfg)
    }
}

fn write_atomic(path: &Path, cfg: &AppConfig) -> Result<()> {
    let text = toml::to_string_pretty(cfg)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    // On Windows, rename onto an existing target requires removing it first
    // because `fs::rename` is not always atomic-replace; do best-effort.
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn merge_json(dst: &mut serde_json::Value, patch: serde_json::Value) {
    use serde_json::Value;
    match (dst, patch) {
        (Value::Object(d), Value::Object(p)) => {
            for (k, v) in p {
                merge_json(d.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, patch_value) => {
            *slot = patch_value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::CURRENT_SCHEMA_VERSION;
    use tokio::sync::broadcast;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "traffic-monitor-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn loads_v2_config_and_upgrades_to_v3() {
        let path = temp_path("v2.toml");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r##"
schemaVersion = 2

[general]
sampleIntervalMs = 1000

[overlay]
visible = true
direction = "row"
align = "left"
padding = 9
items = ["net-down", "cpu"]
fontFamily = "Microsoft YaHei UI"
fontSize = 15
fontWeight = 600
textColor = "#eeeeee"
downColor = "#112233"
upColor = "#445566"
strokeColor = "#000000"
strokeWidth = 1
bgColor = "#101010"
bgOpacity = 0.7
borderRadius = 8
clickThrough = false
doubleClickShowConfig = true

[network]
monitorMode = "all"
monitorInterfaces = []
includeLoopback = false
includeVirtual = false
thresholdEnabled = true
thresholdMonthlyGb = 50
thresholdIface = "all"

[history]
retainDays = 400
enableDiskMonitor = true
enableGpuMonitor = true

[internal]
lastSessionStartedAt = 1
lastThresholdAlertMonth = "2026-05"
"##,
        )
        .unwrap();

        let mgr = ConfigManager::load(path.clone()).unwrap();
        let cfg = mgr.snapshot();

        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            cfg.overlay.items,
            vec![
                crate::config::schema::OverlayItem::NetDown,
                crate::config::schema::OverlayItem::NetUp,
                crate::config::schema::OverlayItem::Cpu,
            ]
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn update_does_not_change_memory_when_persist_fails() {
        let path = temp_path("blocked-dir");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let (tx, _rx) = broadcast::channel(1);
        let mgr = ConfigManager {
            path: path.clone(),
            inner: RwLock::new(AppConfig::default()),
            notify: tx,
        };

        let before = mgr.snapshot();
        let err = mgr.update(|cfg| cfg.general.sample_interval_ms = 1500);

        assert!(err.is_err());
        assert_eq!(mgr.snapshot(), before);

        let _ = std::fs::remove_dir_all(path);
    }
}
