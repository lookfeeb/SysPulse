use crate::error::Result;
use crate::monitor::snapshot::CpuSnapshot;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

pub struct CpuCollector {
    sys: System,
    model: String,
    physical_cores: u32,
}

impl CpuCollector {
    pub fn new() -> Self {
        let mut sys =
            System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()));
        sys.refresh_cpu_all();
        // Sleep briefly then refresh again so the first sample() gets valid usage data.
        // Without this gap, sysinfo returns 0% on the first call.
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_cpu_usage();
        let model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_default();
        let physical_cores = sys.physical_core_count().unwrap_or(0) as u32;
        Self {
            sys,
            model,
            physical_cores,
        }
    }

    pub fn sample(&mut self, expose_per_core: bool) -> Result<CpuSnapshot> {
        self.sys.refresh_cpu_usage();
        let global = self.sys.global_cpu_usage();
        let per_core = if expose_per_core {
            Some(self.sys.cpus().iter().map(|c| c.cpu_usage()).collect())
        } else {
            None
        };
        Ok(CpuSnapshot {
            usage_percent: global,
            per_core,
            model: self.model.clone(),
            physical_cores: self.physical_cores,
        })
    }
}
