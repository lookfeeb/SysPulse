use crate::hw::client::HwClient;
use crate::hw::snapshot::HwSnapshot;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const WATCHDOG_INTERVAL: Duration = Duration::from_secs(2);
const PWM_WRITE_DELTA: f64 = 2.0;
const FUSE_TEMP_C: f64 = 90.0;
const MAX_WRITE_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FanControlMode {
    Bios,
    Manual,
    Curve,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FanCurvePoint {
    pub temp_c: f64,
    pub pwm: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FanControlEntry {
    pub fan_id: String,
    pub mode: FanControlMode,
    pub manual_pwm: f64,
    pub curve: Vec<FanCurvePoint>,
    pub last_written_pwm: Option<f64>,
    pub write_failures: u32,
}

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FanControlStatus {
    pub fuse_hold: bool,
    pub fuse_reason: Option<String>,
    pub max_temp_c: Option<f64>,
    pub entries: Vec<FanControlEntry>,
}

#[derive(Clone, Default)]
pub struct FanControlManager {
    inner: Arc<RwLock<FanControlInner>>,
}

#[derive(Debug, Clone, Default)]
struct FanControlInner {
    entries: HashMap<String, FanControlEntry>,
    fuse_hold: bool,
    fuse_reason: Option<String>,
    max_temp_c: Option<f64>,
}

impl FanControlManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> FanControlStatus {
        let inner = self.inner.read();
        let mut entries: Vec<_> = inner.entries.values().cloned().collect();
        entries.sort_by(|a, b| a.fan_id.cmp(&b.fan_id));
        FanControlStatus {
            fuse_hold: inner.fuse_hold,
            fuse_reason: inner.fuse_reason.clone(),
            max_temp_c: inner.max_temp_c,
            entries,
        }
    }

    pub fn set_manual(&self, fan_id: String, pwm: f64) -> Result<FanControlStatus, String> {
        validate_pwm(pwm)?;
        let mut inner = self.inner.write();
        if inner.fuse_hold {
            return Err("fan control is in fuse hold; reset all controls first".into());
        }
        let entry = inner
            .entries
            .entry(fan_id.clone())
            .or_insert_with(|| FanControlEntry::new(fan_id));
        entry.mode = FanControlMode::Manual;
        entry.manual_pwm = pwm;
        entry.write_failures = 0;
        Ok(status_from_inner(&inner))
    }

    pub fn set_curve(
        &self,
        fan_id: String,
        curve: Vec<FanCurvePoint>,
    ) -> Result<FanControlStatus, String> {
        let curve = normalize_curve(curve)?;
        let mut inner = self.inner.write();
        if inner.fuse_hold {
            return Err("fan control is in fuse hold; reset all controls first".into());
        }
        let entry = inner
            .entries
            .entry(fan_id.clone())
            .or_insert_with(|| FanControlEntry::new(fan_id));
        entry.mode = FanControlMode::Curve;
        entry.curve = curve;
        entry.write_failures = 0;
        Ok(status_from_inner(&inner))
    }

    pub fn reset_entry(&self, fan_id: &str) -> FanControlStatus {
        let mut inner = self.inner.write();
        inner.entries.remove(fan_id);
        status_from_inner(&inner)
    }

    pub fn clear_all(&self) -> FanControlStatus {
        let mut inner = self.inner.write();
        inner.entries.clear();
        inner.fuse_hold = false;
        inner.fuse_reason = None;
        status_from_inner(&inner)
    }
}

impl FanControlEntry {
    fn new(fan_id: String) -> Self {
        Self {
            fan_id,
            mode: FanControlMode::Bios,
            manual_pwm: 50.0,
            curve: default_curve(),
            last_written_pwm: None,
            write_failures: 0,
        }
    }
}

pub fn default_curve() -> Vec<FanCurvePoint> {
    vec![
        FanCurvePoint {
            temp_c: 40.0,
            pwm: 20.0,
        },
        FanCurvePoint {
            temp_c: 55.0,
            pwm: 40.0,
        },
        FanCurvePoint {
            temp_c: 70.0,
            pwm: 70.0,
        },
        FanCurvePoint {
            temp_c: 85.0,
            pwm: 100.0,
        },
    ]
}

pub fn spawn_watchdog(
    app: AppHandle,
    manager: FanControlManager,
    client: Arc<HwClient>,
    last_snapshot: Arc<RwLock<Option<HwSnapshot>>>,
) -> tokio_util::sync::CancellationToken {
    let token = tokio_util::sync::CancellationToken::new();
    let child = token.child_token();
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(WATCHDOG_INTERVAL);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    run_watchdog_once(&app, &manager, &client, &last_snapshot).await;
                }
                _ = child.cancelled() => { break; }
            }
        }
    });
    token
}

pub async fn reset_all_best_effort(manager: &FanControlManager, client: &HwClient) {
    let had_entries = !manager.inner.read().entries.is_empty();
    if had_entries {
        let _ = client.reset_fans().await;
    }
    manager.clear_all();
}

async fn run_watchdog_once(
    app: &AppHandle,
    manager: &FanControlManager,
    client: &HwClient,
    last_snapshot: &Arc<RwLock<Option<HwSnapshot>>>,
) {
    let snapshot = last_snapshot.read().clone();
    let max_temp = snapshot.as_ref().and_then(max_control_temperature);

    {
        let mut inner = manager.inner.write();
        inner.max_temp_c = max_temp;
        if inner.fuse_hold {
            let client = client.clone();
            tauri::async_runtime::spawn(async move {
                let _ = client.reset_fans().await;
            });
            let _ = app.emit("fan-control:changed", status_from_inner(&inner));
            return;
        }
        if max_temp.is_some_and(|t| t >= FUSE_TEMP_C) && !inner.entries.is_empty() {
            inner.fuse_hold = true;
            inner.fuse_reason = Some(format!("temperature reached {:.0}C", max_temp.unwrap()));
        }
    }

    if manager.inner.read().fuse_hold {
        let _ = client.reset_fans().await;
        let _ = app.emit("fan-control:changed", manager.status());
        return;
    }

    let Some(temp) = max_temp else {
        return;
    };

    let entries: Vec<FanControlEntry> = manager.inner.read().entries.values().cloned().collect();
    for entry in entries {
        let target_pwm = match entry.mode {
            FanControlMode::Bios => continue,
            FanControlMode::Manual => entry.manual_pwm,
            FanControlMode::Curve => interpolate_curve(&entry.curve, temp),
        };

        if entry
            .last_written_pwm
            .is_some_and(|prev| (prev - target_pwm).abs() < PWM_WRITE_DELTA)
        {
            continue;
        }

        match client
            .set_fan_manual(entry.fan_id.clone(), target_pwm.clamp(0.0, 100.0))
            .await
        {
            Ok(()) => {
                let mut inner = manager.inner.write();
                if let Some(current) = inner.entries.get_mut(&entry.fan_id) {
                    current.last_written_pwm = Some(target_pwm);
                    current.write_failures = 0;
                }
            }
            Err(e) => {
                let mut should_fuse = false;
                {
                    let mut inner = manager.inner.write();
                    if let Some(current) = inner.entries.get_mut(&entry.fan_id) {
                        current.write_failures += 1;
                        should_fuse = current.write_failures >= MAX_WRITE_FAILURES;
                    }
                    if should_fuse {
                        inner.fuse_hold = true;
                        inner.fuse_reason = Some(format!("fan write failed repeatedly: {e}"));
                    }
                }
                if should_fuse {
                    let _ = client.reset_fans().await;
                    break;
                }
            }
        }
    }

    let _ = app.emit("fan-control:changed", manager.status());
}

pub fn max_control_temperature(snapshot: &HwSnapshot) -> Option<f64> {
    let mut values = Vec::new();
    if let Some(cpu) = &snapshot.cpu {
        values.extend(cpu.package_temp_c);
        values.extend(cpu.per_core_temps_c.iter().flatten().copied());
    }
    for gpu in &snapshot.gpus {
        values.extend(gpu.temp_c);
    }
    values
        .into_iter()
        .filter(|v| v.is_finite())
        .reduce(f64::max)
}

pub fn interpolate_curve(points: &[FanCurvePoint], temp_c: f64) -> f64 {
    if points.is_empty() {
        return 100.0;
    }
    if temp_c <= points[0].temp_c {
        return points[0].pwm;
    }
    for pair in points.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if temp_c <= b.temp_c {
            let span = (b.temp_c - a.temp_c).max(0.1);
            let ratio = ((temp_c - a.temp_c) / span).clamp(0.0, 1.0);
            return a.pwm + (b.pwm - a.pwm) * ratio;
        }
    }
    points.last().map(|p| p.pwm).unwrap_or(100.0)
}

pub(crate) fn normalize_curve(mut curve: Vec<FanCurvePoint>) -> Result<Vec<FanCurvePoint>, String> {
    if curve.len() < 2 {
        return Err("curve needs at least 2 points".into());
    }
    for p in &curve {
        if !p.temp_c.is_finite() || p.temp_c < 0.0 || p.temp_c > 120.0 {
            return Err("curve tempC must be in 0..120".into());
        }
        validate_pwm(p.pwm)?;
    }
    curve.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));
    curve.dedup_by(|a, b| (a.temp_c - b.temp_c).abs() < 0.1);
    if curve.len() < 2 {
        return Err("curve needs at least 2 distinct temperatures".into());
    }
    Ok(curve)
}

fn validate_pwm(pwm: f64) -> Result<(), String> {
    if pwm.is_finite() && (0.0..=100.0).contains(&pwm) {
        Ok(())
    } else {
        Err("pwm must be in 0..100".into())
    }
}

fn status_from_inner(inner: &FanControlInner) -> FanControlStatus {
    let mut entries: Vec<_> = inner.entries.values().cloned().collect();
    entries.sort_by(|a, b| a.fan_id.cmp(&b.fan_id));
    FanControlStatus {
        fuse_hold: inner.fuse_hold,
        fuse_reason: inner.fuse_reason.clone(),
        max_temp_c: inner.max_temp_c,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_interpolates_edges_and_middle() {
        let curve = default_curve();
        assert_eq!(interpolate_curve(&curve, 20.0), 20.0);
        assert_eq!(interpolate_curve(&curve, 85.0), 100.0);
        assert!((interpolate_curve(&curve, 47.5) - 30.0).abs() < 0.01);
        assert_eq!(interpolate_curve(&curve, 100.0), 100.0);
    }

    #[test]
    fn normalize_curve_sorts_and_rejects_bad_pwm() {
        let curve = normalize_curve(vec![
            FanCurvePoint {
                temp_c: 70.0,
                pwm: 80.0,
            },
            FanCurvePoint {
                temp_c: 40.0,
                pwm: 10.0,
            },
        ])
        .unwrap();
        assert_eq!(curve[0].temp_c, 40.0);
        assert!(normalize_curve(vec![
            FanCurvePoint {
                temp_c: 40.0,
                pwm: -1.0,
            },
            FanCurvePoint {
                temp_c: 70.0,
                pwm: 80.0,
            },
        ])
        .is_err());
    }

    #[test]
    fn max_control_temperature_uses_cpu_gpu_sensors() {
        let snap = HwSnapshot {
            cpu: Some(crate::hw::snapshot::CpuHw {
                package_temp_c: Some(60.0),
                per_core_temps_c: vec![Some(63.0)],
                ..Default::default()
            }),
            gpus: vec![crate::hw::snapshot::GpuHw {
                temp_c: Some(72.0),
                ..Default::default()
            }],
            disks: vec![crate::hw::snapshot::DiskHw {
                temp_c: Some(88.0),
                ..Default::default()
            }],
            motherboard: Some(crate::hw::snapshot::MotherboardHw {
                temperatures_c: vec![crate::hw::snapshot::NamedValue {
                    value: 50.0,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(max_control_temperature(&snap), Some(72.0));
    }
}
