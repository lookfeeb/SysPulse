use crate::config::ConfigManager;
use crate::monitor::{MonitorRegistry, Snapshot};
use crate::storage::{TrafficDelta, WriterHandle};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{interval, MissedTickBehavior};

// ─── Adaptive Interval Engine ───────────────────────────────────────────────
//
// Multi-signal EWMA-based adaptive sampling algorithm.
//
// Design goals:
//   1. React quickly to bursts (network spikes, CPU jumps)
//   2. Decay slowly to conserve resources during idle
//   3. Avoid oscillation (hysteresis + smoothing)
//   4. Minimal per-tick overhead (no allocations, O(1) math)
//
// Signals tracked (each with independent EWMA):
//   - CPU usage (0–100%)
//   - Network throughput (bytes/sec, combined up+down)
//   - Rate-of-change of CPU (derivative, detects sudden jumps)
//   - Rate-of-change of network (derivative)
//
// The final interval is computed as:
//   activity_score = weighted_blend(cpu_signal, net_signal, cpu_delta, net_delta)
//   target_interval = MAX - (MAX - MIN) * activity_score^CURVE_EXP
//   actual_interval = ewma_smooth(current, target, SMOOTHING_FACTOR)
//   actual_interval = clamp(actual_interval, MIN, MAX)
//
// The exponential curve (CURVE_EXP < 1) makes the system more responsive
// at low activity levels, ensuring even moderate load triggers faster sampling.

const ADAPTIVE_MIN_MS: u64 = 500;
const ADAPTIVE_MAX_MS: u64 = 2000;

/// EWMA smoothing factor for individual signals (higher = more responsive)
const SIGNAL_ALPHA: f64 = 0.3;

/// EWMA smoothing factor for the output interval (lower = smoother transitions)
const INTERVAL_ALPHA_UP: f64 = 0.6;   // speed up quickly
const INTERVAL_ALPHA_DOWN: f64 = 0.15; // slow down gradually (hysteresis)

/// Curve exponent: <1 makes it more aggressive at low activity
const CURVE_EXP: f64 = 0.7;

/// Signal weights (must sum to 1.0)
const W_CPU: f64 = 0.30;
const W_NET: f64 = 0.30;
const W_CPU_DELTA: f64 = 0.20;
const W_NET_DELTA: f64 = 0.20;

/// Normalization constants
const CPU_FULL: f64 = 100.0;
const NET_FULL: f64 = 10_000_000.0; // 10 MB/s = "full" activity
const DELTA_CPU_FULL: f64 = 30.0;   // 30% jump per tick = max delta signal
const DELTA_NET_FULL: f64 = 5_000_000.0; // 5 MB/s change per tick = max

struct AdaptiveEngine {
    /// Smoothed CPU usage (0..1)
    ewma_cpu: f64,
    /// Smoothed network throughput (0..1)
    ewma_net: f64,
    /// Smoothed CPU rate-of-change (0..1)
    ewma_cpu_delta: f64,
    /// Smoothed network rate-of-change (0..1)
    ewma_net_delta: f64,
    /// Previous raw CPU value for delta computation
    prev_cpu: f64,
    /// Previous raw net value for delta computation
    prev_net: f64,
    /// Current smoothed interval in ms (f64 for precision)
    smoothed_interval: f64,
    /// Whether we have at least one prior sample
    initialized: bool,
}

impl AdaptiveEngine {
    fn new() -> Self {
        Self {
            ewma_cpu: 0.0,
            ewma_net: 0.0,
            ewma_cpu_delta: 0.0,
            ewma_net_delta: 0.0,
            prev_cpu: 0.0,
            prev_net: 0.0,
            smoothed_interval: ADAPTIVE_MAX_MS as f64,
            initialized: false,
        }
    }

    /// Feed a new snapshot and return the recommended interval in ms.
    fn update(&mut self, snap: &Snapshot) -> u64 {
        let raw_cpu = snap.cpu.usage_percent as f64;
        let raw_net = (snap.network.total.bytes_recv_per_sec
            + snap.network.total.bytes_sent_per_sec) as f64;

        if !self.initialized {
            // First sample: initialize without delta
            self.ewma_cpu = (raw_cpu / CPU_FULL).min(1.0);
            self.ewma_net = (raw_net / NET_FULL).min(1.0);
            self.prev_cpu = raw_cpu;
            self.prev_net = raw_net;
            self.initialized = true;
            // Start at a middle ground
            self.smoothed_interval = (ADAPTIVE_MIN_MS + ADAPTIVE_MAX_MS) as f64 / 2.0;
            return self.smoothed_interval as u64;
        }

        // Compute deltas (absolute change since last tick)
        let delta_cpu = (raw_cpu - self.prev_cpu).abs();
        let delta_net = (raw_net - self.prev_net).abs();
        self.prev_cpu = raw_cpu;
        self.prev_net = raw_net;

        // Update EWMAs (normalize to 0..1 range)
        self.ewma_cpu = ewma(self.ewma_cpu, (raw_cpu / CPU_FULL).min(1.0), SIGNAL_ALPHA);
        self.ewma_net = ewma(self.ewma_net, (raw_net / NET_FULL).min(1.0), SIGNAL_ALPHA);
        self.ewma_cpu_delta = ewma(
            self.ewma_cpu_delta,
            (delta_cpu / DELTA_CPU_FULL).min(1.0),
            SIGNAL_ALPHA,
        );
        self.ewma_net_delta = ewma(
            self.ewma_net_delta,
            (delta_net / DELTA_NET_FULL).min(1.0),
            SIGNAL_ALPHA,
        );

        // Weighted activity score (0..1)
        let score = (W_CPU * self.ewma_cpu
            + W_NET * self.ewma_net
            + W_CPU_DELTA * self.ewma_cpu_delta
            + W_NET_DELTA * self.ewma_net_delta)
            .clamp(0.0, 1.0);

        // Apply curve: score^0.7 makes moderate activity (0.3) map to ~0.41
        let curved = score.powf(CURVE_EXP);

        // Target interval: high activity → MIN, low activity → MAX
        let range = (ADAPTIVE_MAX_MS - ADAPTIVE_MIN_MS) as f64;
        let target = ADAPTIVE_MAX_MS as f64 - range * curved;

        // Asymmetric smoothing: speed up fast, slow down gradually
        let alpha = if target < self.smoothed_interval {
            INTERVAL_ALPHA_UP
        } else {
            INTERVAL_ALPHA_DOWN
        };
        self.smoothed_interval = ewma(self.smoothed_interval, target, alpha);

        // Quantize to 50ms steps to avoid excessive timer resets
        let quantized = ((self.smoothed_interval / 50.0).round() * 50.0) as u64;
        quantized.clamp(ADAPTIVE_MIN_MS, ADAPTIVE_MAX_MS)
    }
}

#[inline]
fn ewma(prev: f64, new_val: f64, alpha: f64) -> f64 {
    prev + alpha * (new_val - prev)
}

pub enum SamplerCmd {
    Pause,
    Resume,
    Shutdown,
}

#[derive(Clone)]
pub struct SamplerHandle {
    cmd_tx: mpsc::Sender<SamplerCmd>,
    pub bus: broadcast::Sender<Snapshot>,
}

impl SamplerHandle {
    pub async fn pause(&self) {
        let _ = self.cmd_tx.send(SamplerCmd::Pause).await;
    }
    pub async fn resume(&self) {
        let _ = self.cmd_tx.send(SamplerCmd::Resume).await;
    }
    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(SamplerCmd::Shutdown).await;
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Snapshot> {
        self.bus.subscribe()
    }
}

pub fn spawn(config: Arc<ConfigManager>, writer: WriterHandle) -> SamplerHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let (bus_tx, _) = broadcast::channel::<Snapshot>(32);
    let bus_for_task = bus_tx.clone();
    let mut cfg_rx = config.subscribe();

    tauri::async_runtime::spawn(async move {
        let initial_cfg = config.snapshot();
        let mut registry = match tokio::task::spawn_blocking({
            let cfg = initial_cfg.clone();
            move || MonitorRegistry::build(&cfg)
        }).await.unwrap() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(?e, "failed to build collectors");
                return;
            }
        };
        let mut current_cfg = initial_cfg;
        let mut current_interval_ms = if current_cfg.general.adaptive_interval {
            ADAPTIVE_MAX_MS
        } else {
            current_cfg.general.sample_interval_ms as u64
        };
        let mut tick = interval(Duration::from_millis(current_interval_ms));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut paused = false;
        let mut adaptive_engine = AdaptiveEngine::new();

        // Immediately perform first sample so UI gets data ASAP (don't wait for first tick)
        {
            let snap = registry.sample(Instant::now(), &current_cfg);
            let _ = bus_for_task.send(snap);
        }

        // Per-luid prev totals to compute deltas for persistence (separate from
        // per-second rate computation in the collector).
        let mut prev_totals: HashMap<u64, (u64, u64)> = HashMap::new();

        loop {
            tokio::select! {
                _ = tick.tick(), if !paused => {
                    let snap = registry.sample(Instant::now(), &current_cfg);

                    // Adaptive interval: use EWMA engine to compute optimal interval
                    if current_cfg.general.adaptive_interval {
                        let target_ms = adaptive_engine.update(&snap);
                        if target_ms != current_interval_ms {
                            current_interval_ms = target_ms;
                            tick = interval(Duration::from_millis(current_interval_ms));
                            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
                            tracing::debug!(current_interval_ms, "adaptive interval adjusted");
                        }
                    }

                    // Persist deltas (per interface, per local-day bucket).
                    let today = chrono::Local::now().date_naive();
                    for iface in &snap.network.interfaces {
                        let prev = prev_totals.get(&iface.luid).copied();
                        let (drecv, dsent) = match prev {
                            Some((r, s)) => (
                                iface.accepted_bytes_recv_total.saturating_sub(r),
                                iface.accepted_bytes_sent_total.saturating_sub(s),
                            ),
                            None => (0, 0),
                        };
                        prev_totals.insert(
                            iface.luid,
                            (iface.accepted_bytes_recv_total, iface.accepted_bytes_sent_total),
                        );
                        if drecv > 0 || dsent > 0 {
                            writer.try_send_delta(TrafficDelta {
                                luid: iface.luid,
                                date_local: today,
                                bytes_recv: drecv,
                                bytes_sent: dsent,
                            });
                        }
                    }

                    let _ = bus_for_task.send(snap);
                }
                Ok(new_cfg) = cfg_rx.recv() => {
                    // Reset interval if mode changed or fixed interval changed
                    let mode_changed = new_cfg.general.adaptive_interval != current_cfg.general.adaptive_interval;
                    let fixed_changed = !new_cfg.general.adaptive_interval
                        && new_cfg.general.sample_interval_ms != current_cfg.general.sample_interval_ms;

                    if mode_changed || fixed_changed {
                        if new_cfg.general.adaptive_interval {
                            // Switching to adaptive: reset engine
                            adaptive_engine = AdaptiveEngine::new();
                            current_interval_ms = ADAPTIVE_MAX_MS;
                        } else {
                            current_interval_ms = new_cfg.general.sample_interval_ms as u64;
                        }
                        tick = interval(Duration::from_millis(current_interval_ms));
                        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        tracing::info!(current_interval_ms, adaptive = new_cfg.general.adaptive_interval, "sampler: interval changed");
                    }
                    if let Err(e) = registry.reconfigure(&new_cfg) {
                        tracing::warn!(?e, "registry reconfigure failed");
                    }
                    current_cfg = new_cfg;
                }
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        SamplerCmd::Pause => { paused = true; tracing::info!("sampler paused"); }
                        SamplerCmd::Resume => { paused = false; tracing::info!("sampler resumed"); }
                        SamplerCmd::Shutdown => { tracing::info!("sampler shutdown"); break; }
                    }
                }
                else => { break; }
            }
        }
    });

    SamplerHandle {
        cmd_tx,
        bus: bus_tx,
    }
}
