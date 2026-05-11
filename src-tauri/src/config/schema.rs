use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayItem {
    NetDown,
    NetUp,
    Cpu,
    CpuFreq,
    Mem,
    DiskRead,
    DiskWrite,
    Gpu,
    CpuTemp,
    GpuTemp,
    GpuUsage,
    DiskTemp,
    FanRpm,
    MbTemp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMonitorMode {
    All,
    Specified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneralConfig {
    pub sample_interval_ms: u32,
    /// When true, the sampler dynamically adjusts the interval based on system activity.
    /// Active (CPU>50% or net>1MB/s) → 500ms, idle → 2000ms.
    /// The `sample_interval_ms` field is ignored when adaptive is enabled.
    pub adaptive_interval: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            sample_interval_ms: 1000,
            adaptive_interval: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct OverlayConfig {
    pub items: Vec<OverlayItem>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            items: vec![
                OverlayItem::NetDown,
                OverlayItem::NetUp,
                OverlayItem::Cpu,
                OverlayItem::Mem,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct NetworkConfig {
    pub monitor_mode: NetworkMonitorMode,
    pub monitor_interfaces: Vec<String>,
    pub include_loopback: bool,
    pub include_virtual: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            monitor_mode: NetworkMonitorMode::All,
            monitor_interfaces: vec![],
            include_loopback: false,
            include_virtual: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryConfig {
    pub retain_days: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { retain_days: 400 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    pub schema_version: u32,
    pub general: GeneralConfig,
    pub overlay: OverlayConfig,
    pub network: NetworkConfig,
    pub history: HistoryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            general: GeneralConfig::default(),
            overlay: OverlayConfig::default(),
            network: NetworkConfig::default(),
            history: HistoryConfig::default(),
        }
    }
}

pub const CURRENT_SCHEMA_VERSION: u32 = 4;
