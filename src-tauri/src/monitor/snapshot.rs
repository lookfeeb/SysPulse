use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CpuSnapshot {
    pub usage_percent: f32,
    pub per_core: Option<Vec<f32>>,
    pub model: String,
    pub physical_cores: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub used_percent: f32,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceStats {
    pub luid: u64,
    pub name: String,
    pub description: String,
    pub is_up: bool,
    pub is_physical: bool,
    pub bytes_sent_total: u64,
    pub bytes_recv_total: u64,
    pub accepted_bytes_sent_total: u64,
    pub accepted_bytes_recv_total: u64,
    pub bytes_sent_per_sec: u64,
    pub bytes_recv_per_sec: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    pub interfaces: Vec<InterfaceStats>,
    pub total: InterfaceStats,
    pub sample_interval_ms: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub timestamp_ms: i64,
    pub cpu: CpuSnapshot,
    pub memory: MemorySnapshot,
    pub network: NetworkSnapshot,
}
