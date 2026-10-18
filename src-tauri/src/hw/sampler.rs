use crate::hw::client::HwClient;
use crate::hw::snapshot::HwSnapshot;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{sleep_until, Instant};

fn next_sample_delay(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.min(3))
}

#[derive(Clone)]
pub struct HwSamplerHandle {
    shutdown_tx: mpsc::Sender<()>,
    pub bus: broadcast::Sender<HwSnapshot>,
}

impl HwSamplerHandle {
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
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
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(8);
    let (bus_tx, _) = broadcast::channel::<HwSnapshot>(16);
    let bus_clone = bus_tx.clone();

    tauri::async_runtime::spawn(async move {
        let mut next = Instant::now();
        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::select! {
                _ = sleep_until(next) => {
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
                    // Rest after completion: slow hardware never causes catch-up bursts.
                    next = Instant::now() + next_sample_delay(consecutive_failures);
                }
                Some(()) = shutdown_rx.recv() => {
                    break;
                }
                else => { break; }
            }
        }
    });

    HwSamplerHandle {
        shutdown_tx,
        bus: bus_tx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_collection_backs_off_with_a_bound_and_success_resets() {
        assert_eq!(next_sample_delay(0), Duration::from_secs(1));
        assert_eq!(next_sample_delay(1), Duration::from_secs(2));
        assert_eq!(next_sample_delay(2), Duration::from_secs(4));
        assert_eq!(next_sample_delay(u32::MAX), Duration::from_secs(8));
    }
}
