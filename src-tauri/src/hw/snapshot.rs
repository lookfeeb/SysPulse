use serde::{Deserialize, Serialize};

// Mirrors hw-helper/HwSnapshot.cs. Keep the two in sync.

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HwSnapshot {
    /// Local monotonic receipt time; never accepted from IPC or persisted.
    #[serde(skip)]
    #[specta(skip)]
    pub received_at: Option<std::time::Instant>,
    pub timestamp_ms: i64,
    #[serde(default)]
    pub sample_age_ms: u64,
    #[serde(default)]
    pub storage_age_ms: u64,
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
    pub per_core_usage: Vec<Option<f64>>,
    pub total_usage: Option<f64>,
    pub frequency_mhz: Option<f64>,
    pub power_w: Option<f64>,
}

impl HwSnapshot {
    /// Account for time spent in the backend cache before handing data to a
    /// new UI subscriber. Reading the cache must not rejuvenate the sample.
    pub fn for_display(&self) -> Self {
        let now = std::time::Instant::now();
        let elapsed = self.received_at.map_or(u64::MAX, |at| {
            now.saturating_duration_since(at)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        });
        let mut snapshot = self.clone();
        snapshot.sample_age_ms = snapshot.sample_age_ms.saturating_add(elapsed);
        snapshot.storage_age_ms = snapshot.storage_age_ms.saturating_add(elapsed);
        snapshot.received_at = Some(now);
        snapshot
    }

    pub fn is_fresh(&self) -> bool {
        self.received_at.is_some_and(|at| {
            self.sample_age_ms
                .saturating_add(at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
                <= 5000
        })
    }

    pub fn is_connected(&self) -> bool {
        self.received_at
            .is_some_and(|at| at.elapsed() <= std::time::Duration::from_secs(5))
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;
    #[test]
    fn cached_snapshot_preserves_age_for_display_and_control() {
        let sample = HwSnapshot {
            received_at: Some(std::time::Instant::now() - std::time::Duration::from_secs(3)),
            sample_age_ms: 3000,
            storage_age_ms: 28000,
            ..Default::default()
        };
        let display = sample.for_display();
        assert!(display.sample_age_ms >= 6000);
        assert!(display.storage_age_ms >= 31000);
        assert!(!display.is_fresh());
        assert_eq!(sample.sample_age_ms, 3000);
        assert!(!HwSnapshot::default().for_display().is_fresh());
    }
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
