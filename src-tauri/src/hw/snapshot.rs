use serde::{Deserialize, Serialize};

// Mirrors hw-helper/HwSnapshot.cs. Keep the two in sync.

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HwSnapshot {
    pub timestamp_ms: i64,
    pub cpu: Option<CpuHw>,
    #[serde(default)]
    #[specta(optional = false)]
    pub gpus: Vec<GpuHw>,
    pub memory: Option<MemoryHw>,
    #[serde(default)]
    #[specta(optional = false)]
    pub disks: Vec<DiskHw>,
    pub motherboard: Option<MotherboardHw>,
    #[serde(default)]
    #[specta(optional = false)]
    pub fans: Vec<FanHw>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CpuHw {
    pub name: String,
    pub package_temp_c: Option<f64>,
    #[serde(default)]
    #[specta(optional = false)]
    pub per_core_temps_c: Vec<Option<f64>>,
    #[serde(default)]
    #[specta(optional = false)]
    pub per_core_usage: Vec<f64>,
    pub total_usage: f64,
    pub frequency_mhz: Option<f64>,
    pub power_w: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GpuHw {
    pub index: u32,
    pub name: String,
    pub vendor: String,
    pub usage_percent: Option<f64>,
    pub mem_used_mb: Option<f64>,
    pub mem_total_mb: Option<f64>,
    pub temp_c: Option<f64>,
    pub power_w: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub fan_pwm: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHw {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub used_percent: f64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    #[serde(default)]
    #[specta(optional = false)]
    pub modules: Vec<MemoryModule>,
    pub frequency_mhz: Option<f64>,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryModule {
    pub slot: String,
    pub capacity_bytes: u64,
    pub speed_mtps: Option<f64>,
    pub manufacturer: String,
    pub part_number: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiskHw {
    pub index: u32,
    pub model: String,
    pub bus: String,
    pub temp_c: Option<f64>,
    pub health: String,
    pub read_bytes_per_sec: Option<f64>,
    pub write_bytes_per_sec: Option<f64>,
    pub total_bytes: u64,
    pub used_bytes: Option<u64>,
    pub identifier: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MotherboardHw {
    pub vendor: String,
    pub model: String,
    pub super_io: Option<String>,
    #[serde(default)]
    #[specta(optional = false)]
    pub temperatures_c: Vec<NamedValue>,
    #[serde(default)]
    #[specta(optional = false)]
    pub voltages_v: Vec<NamedValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NamedValue {
    pub name: String,
    pub value: f64,
    #[serde(default)]
    #[specta(optional = false)]
    pub identifier: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FanHw {
    pub id: String,
    pub name: String,
    pub rpm: Option<f64>,
    pub pwm_percent: Option<f64>,
    pub controllable: bool,
}
