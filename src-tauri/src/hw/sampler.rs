use crate::hw::client::HwClient;
use crate::hw::snapshot::HwSnapshot;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, MissedTickBehavior};

pub enum HwSamplerCmd {
    Pause,
    Resume,
    Shutdown,
}

#[derive(Clone)]
pub struct HwSamplerHandle {
    cmd_tx: mpsc::Sender<HwSamplerCmd>,
    pub bus: broadcast::Sender<HwSnapshot>,
}

impl HwSamplerHandle {
    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(HwSamplerCmd::Shutdown).await;
    }
    pub fn subscribe(&self) -> broadcast::Receiver<HwSnapshot> {
        self.bus.subscribe()
    }
}

/// Spawn a 1Hz hardware sampler that asks the helper for a fresh snapshot
/// each tick and broadcasts successful samples to subscribers. Failures are
/// reported through helper status/logs and do not overwrite the last good UI
/// snapshot with empty data.
pub fn spawn(client: Arc<HwClient>) -> HwSamplerHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<HwSamplerCmd>(8);
    let (bus_tx, _) = broadcast::channel::<HwSnapshot>(16);
    let bus_clone = bus_tx.clone();

    tauri::async_runtime::spawn(async move {
        let mut tick = interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut paused = false;
        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::select! {
                _ = tick.tick(), if !paused => {
                    match client.snapshot().await {
                        Ok(snap) => {
                            consecutive_failures = 0;
                            let _ = bus_clone.send(snap);
                        }
                        Err(e) => {
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            // Throttle the warn so we don't spam the log.
                            if consecutive_failures <= 3 || consecutive_failures % 30 == 0 {
                                tracing::warn!("hw snapshot failed (#{consecutive_failures}): {e}");
                            }
                        }
                    }
                }
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        HwSamplerCmd::Pause => { paused = true; }
                        HwSamplerCmd::Resume => { paused = false; }
                        HwSamplerCmd::Shutdown => { break; }
                    }
                }
                else => { break; }
            }
        }
    });

    HwSamplerHandle {
        cmd_tx,
        bus: bus_tx,
    }
}
