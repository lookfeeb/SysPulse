use crate::config::schema::{NetworkConfig, NetworkMonitorMode};
use crate::error::Result;
use crate::monitor::snapshot::{InterfaceStats, NetworkSnapshot};
use std::collections::HashMap;
use std::time::Instant;

pub struct NetworkCollector {
    last_at: Option<Instant>,
    last_bytes: HashMap<u64, (u64, u64)>, // luid -> (in, out)
}

impl NetworkCollector {
    pub fn new() -> Self {
        Self {
            last_at: None,
            last_bytes: HashMap::new(),
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
            // Only count physical adapters (Ethernet / WiFi) that are currently up.
            // This excludes loopback, virtual adapters (VMware, Hyper-V, VPN tunnels, etc.)
            // which can produce erratic counter values.
            if !row.is_physical {
                return false;
            }
            if !row.is_up {
                return false;
            }
            true
        };

        let in_monitor_set = |row: &Row| -> bool {
            match cfg.monitor_mode {
                NetworkMonitorMode::All => allow_iface(row),
                NetworkMonitorMode::Specified => {
                    let key = row.luid.to_string();
                    cfg.monitor_interfaces.iter().any(|s| s == &key)
                }
            }
        };

        for row in rows {
            current_luids.insert(row.luid);
            let prev = self.last_bytes.get(&row.luid).copied();
            let (rate_in, rate_out) = match prev {
                Some((pi, po)) if secs > 0.0 => {
                    let di = row.bytes_in.saturating_sub(pi);
                    let do_ = row.bytes_out.saturating_sub(po);
                    ((di as f64 / secs) as u64, (do_ as f64 / secs) as u64)
                }
                _ => (0u64, 0u64),
            };

            self.last_bytes
                .insert(row.luid, (row.bytes_in, row.bytes_out));

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
        self.last_bytes.retain(|k, _| current_luids.contains(k));
        self.last_at = Some(now);

        Ok(NetworkSnapshot {
            interfaces,
            total,
            sample_interval_ms: elapsed_ms,
        })
    }
}

#[derive(Debug, Clone)]
struct Row {
    luid: u64,
    name: String,
    description: String,
    is_up: bool,
    is_loopback: bool,
    is_physical: bool,
    bytes_in: u64,
    bytes_out: u64,
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
            bytes_in: r.bytes_in,
            bytes_out: r.bytes_out,
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
            bytes_in: data.total_received(),
            bytes_out: data.total_transmitted(),
        });
        next_luid += 1;
    }
    Ok(out)
}
