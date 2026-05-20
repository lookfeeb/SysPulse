use crate::config::schema::{NetworkConfig, NetworkMonitorMode};
use crate::error::Result;
use crate::monitor::snapshot::{InterfaceStats, NetworkSnapshot};
use std::collections::HashMap;
use std::time::Instant;

const MIN_RATE_SAMPLE_MS: u32 = 200;
const DEFAULT_MAX_BYTES_PER_SEC: u64 = 125 * 1024 * 1024; // 1 Gbps fallback for unknown links.
const RATE_HEADROOM_NUMERATOR: u64 = 115;
const RATE_HEADROOM_DENOMINATOR: u64 = 100;

pub struct NetworkCollector {
    last_at: Option<Instant>,
    last_state: HashMap<u64, LastIfaceState>,
}

impl NetworkCollector {
    pub fn new() -> Self {
        Self {
            last_at: None,
            last_state: HashMap::new(),
        }
    }

    pub fn sample(&mut self, now: Instant, cfg: &NetworkConfig) -> Result<NetworkSnapshot> {
        let rows = read_rows()?;
        let elapsed_ms = match self.last_at {
            Some(prev) => now.duration_since(prev).as_millis().max(1) as u32,
            None => 0,
        };
        let secs = if elapsed_ms == 0 {
            0.0
        } else {
            elapsed_ms as f64 / 1000.0
        };

        let mut current_luids: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut interfaces = Vec::with_capacity(rows.len());
        let mut total = InterfaceStats {
            name: "<all>".into(),
            description: "All interfaces".into(),
            ..Default::default()
        };

        let allow_iface = |row: &Row| -> bool {
            // Include any interface that is:
            // - currently Up
            // - allowed by loopback / virtual adapter config
            // - not a tunnel/PPP (VPN)
            if !row.is_up {
                return false;
            }
            if row.is_tunnel {
                return false;
            }
            if row.is_loopback {
                return cfg.include_loopback;
            }
            row.is_physical || cfg.include_virtual
        };

        let in_monitor_set = |row: &Row| -> bool {
            match cfg.monitor_mode {
                NetworkMonitorMode::All => allow_iface(row),
                NetworkMonitorMode::Specified => {
                    let key = row.luid.to_string();
                    allow_iface(row) && cfg.monitor_interfaces.iter().any(|s| s == &key)
                }
            }
        };

        for row in rows {
            current_luids.insert(row.luid);
            let prev = self.last_state.get(&row.luid).copied();
            let (rate_in, rate_out, accepted_in, accepted_out) =
                compute_rates(&row, prev, elapsed_ms, secs);

            self.last_state.insert(
                row.luid,
                LastIfaceState {
                    bytes_in: row.bytes_in,
                    bytes_out: row.bytes_out,
                    rate_in,
                    rate_out,
                    accepted_bytes_in: accepted_in,
                    accepted_bytes_out: accepted_out,
                },
            );

            if !allow_iface(&row) {
                continue;
            }

            let stat = InterfaceStats {
                luid: row.luid,
                name: row.name.clone(),
                description: row.description.clone(),
                is_up: row.is_up,
                is_physical: row.is_physical,
                bytes_sent_total: row.bytes_out,
                bytes_recv_total: row.bytes_in,
                accepted_bytes_sent_total: accepted_out,
                accepted_bytes_recv_total: accepted_in,
                bytes_sent_per_sec: rate_out,
                bytes_recv_per_sec: rate_in,
            };

            if in_monitor_set(&row) {
                total.bytes_sent_total = total.bytes_sent_total.saturating_add(row.bytes_out);
                total.bytes_recv_total = total.bytes_recv_total.saturating_add(row.bytes_in);
                total.bytes_sent_per_sec = total.bytes_sent_per_sec.saturating_add(rate_out);
                total.bytes_recv_per_sec = total.bytes_recv_per_sec.saturating_add(rate_in);
            }

            interfaces.push(stat);
        }

        // Drop disappeared interfaces from the prev map.
        self.last_state.retain(|k, _| current_luids.contains(k));
        self.last_at = Some(now);

        Ok(NetworkSnapshot {
            interfaces,
            total,
            sample_interval_ms: elapsed_ms,
        })
    }
}

impl Default for NetworkCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
struct LastIfaceState {
    bytes_in: u64,
    bytes_out: u64,
    rate_in: u64,
    rate_out: u64,
    accepted_bytes_in: u64,
    accepted_bytes_out: u64,
}

#[derive(Debug, Clone)]
struct Row {
    luid: u64,
    name: String,
    description: String,
    is_up: bool,
    is_loopback: bool,
    is_physical: bool,
    is_tunnel: bool,
    bytes_in: u64,
    bytes_out: u64,
    in_speed_bps: u64,
    out_speed_bps: u64,
}

fn compute_rates(
    row: &Row,
    prev: Option<LastIfaceState>,
    elapsed_ms: u32,
    secs: f64,
) -> (u64, u64, u64, u64) {
    let Some(prev) = prev else {
        return (0, 0, row.bytes_in, row.bytes_out);
    };

    // Windows counters can reset when adapters sleep, roam, reconnect, or get
    // recreated. Treat rollback as a baseline reset instead of a huge delta.
    if row.bytes_in < prev.bytes_in || row.bytes_out < prev.bytes_out {
        return (0, 0, row.bytes_in, row.bytes_out);
    }

    if elapsed_ms < MIN_RATE_SAMPLE_MS || secs <= 0.0 {
        return (
            prev.rate_in,
            prev.rate_out,
            prev.accepted_bytes_in,
            prev.accepted_bytes_out,
        );
    }

    let delta_in = row.bytes_in - prev.bytes_in;
    let delta_out = row.bytes_out - prev.bytes_out;
    let rate_in = (delta_in as f64 / secs) as u64;
    let rate_out = (delta_out as f64 / secs) as u64;
    let max_in = max_plausible_bytes_per_sec(row.in_speed_bps);
    let max_out = max_plausible_bytes_per_sec(row.out_speed_bps);

    if rate_in > max_in || rate_out > max_out {
        tracing::debug!(
            luid = row.luid,
            name = %row.name,
            elapsed_ms,
            rate_in,
            rate_out,
            max_in,
            max_out,
            "ignored implausible network rate sample"
        );
        return (
            prev.rate_in,
            prev.rate_out,
            prev.accepted_bytes_in,
            prev.accepted_bytes_out,
        );
    }

    (rate_in, rate_out, row.bytes_in, row.bytes_out)
}

fn max_plausible_bytes_per_sec(link_speed_bps: u64) -> u64 {
    let base = if link_speed_bps > 0 {
        link_speed_bps / 8
    } else {
        DEFAULT_MAX_BYTES_PER_SEC
    };
    base.saturating_mul(RATE_HEADROOM_NUMERATOR) / RATE_HEADROOM_DENOMINATOR
}

#[cfg(windows)]
fn read_rows() -> Result<Vec<Row>> {
    let rows = crate::windows_api::if_table::list_interfaces()?;
    Ok(rows
        .into_iter()
        .map(|r| Row {
            luid: r.luid,
            name: r.name,
            description: r.description,
            is_up: r.is_up,
            is_loopback: r.is_loopback,
            is_physical: r.is_physical,
            is_tunnel: r.is_tunnel,
            bytes_in: r.bytes_in,
            bytes_out: r.bytes_out,
            in_speed_bps: r.in_speed_bps,
            out_speed_bps: r.out_speed_bps,
        })
        .collect())
}

#[cfg(not(windows))]
fn read_rows() -> Result<Vec<Row>> {
    use sysinfo::Networks;
    let mut nets = Networks::new_with_refreshed_list();
    nets.refresh(true);
    let mut out = Vec::new();
    let mut next_luid: u64 = 1;
    for (name, data) in &nets {
        out.push(Row {
            luid: next_luid,
            name: name.clone(),
            description: name.clone(),
            is_up: true,
            is_loopback: name.starts_with("lo"),
            is_physical: !(name.starts_with("docker") || name.starts_with("veth")),
            is_tunnel: false,
            bytes_in: data.total_received(),
            bytes_out: data.total_transmitted(),
            in_speed_bps: 0,
            out_speed_bps: 0,
        });
        next_luid += 1;
    }
    Ok(out)
}
