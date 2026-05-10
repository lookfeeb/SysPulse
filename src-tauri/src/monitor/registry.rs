use crate::config::schema::AppConfig;
use crate::error::Result;
use crate::monitor::cpu::CpuCollector;
use crate::monitor::memory::MemoryCollector;
use crate::monitor::network::NetworkCollector;
use crate::monitor::snapshot::Snapshot;
use std::time::Instant;

pub struct MonitorRegistry {
    cpu: CpuCollector,
    mem: MemoryCollector,
    net: NetworkCollector,
}

impl MonitorRegistry {
    pub fn build(_cfg: &AppConfig) -> Result<Self> {
        Ok(Self {
            cpu: CpuCollector::new(),
            mem: MemoryCollector::new(),
            net: NetworkCollector::new(),
        })
    }

    pub fn sample(&mut self, now: Instant, cfg: &AppConfig) -> Snapshot {
        let cpu = self.cpu.sample(false).unwrap_or_default();
        let memory = self.mem.sample().unwrap_or_default();
        let network = self.net.sample(now, &cfg.network).unwrap_or_default();
        Snapshot {
            timestamp_ms: chrono::Local::now().timestamp_millis(),
            cpu,
            memory,
            network,
        }
    }

    pub fn reconfigure(&mut self, _cfg: &AppConfig) -> Result<()> {
        // Network/CPU/Memory don't need rebuilding; their config is applied per-sample.
        Ok(())
    }
}
