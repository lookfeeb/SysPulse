use crate::error::Result;
use crate::monitor::snapshot::MemorySnapshot;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};

pub struct MemoryCollector {
    sys: System,
}

impl MemoryCollector {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::new().with_memory(MemoryRefreshKind::everything()),
        );
        Self { sys }
    }

    pub fn sample(&mut self) -> Result<MemorySnapshot> {
        self.sys.refresh_memory();
        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        let percent = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };
        Ok(MemorySnapshot {
            total_bytes: total,
            used_bytes: used,
            used_percent: percent,
            swap_total_bytes: self.sys.total_swap(),
            swap_used_bytes: self.sys.used_swap(),
        })
    }
}
