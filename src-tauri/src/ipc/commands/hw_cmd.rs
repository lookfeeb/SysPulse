use crate::app::AppState;
use crate::error::{AppError, IpcError};
use crate::hw::client::HelperStatus;
use crate::hw::fan_control::{fresh_control_temperature, interpolate_curve, normalize_curve};
use crate::hw::{admin, FanControlStatus, FanCurvePoint, HwSnapshot};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
#[specta::specta]
pub fn get_hw_snapshot(state: State<'_, AppState>) -> Result<Option<HwSnapshot>, IpcError> {
    Ok(state
        .last_hw_snapshot
        .read()
        .as_ref()
        .filter(|s| s.is_fresh())
        .cloned())
}

#[tauri::command]
#[specta::specta]
pub fn get_helper_status(state: State<'_, AppState>) -> Result<HelperStatus, IpcError> {
    Ok(state.hw_client.status())
}

#[tauri::command]
#[specta::specta]
pub fn is_admin() -> Result<bool, IpcError> {
    Ok(admin::is_elevated())
}

#[tauri::command]
#[specta::specta]
pub fn get_fan_control_state(state: State<'_, AppState>) -> Result<FanControlStatus, IpcError> {
    Ok(state.fan_control.status())
}

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SetFanManualArgs {
    pub fan_id: String,
    pub pwm: f64,
}

#[tauri::command]
#[specta::specta]
pub async fn set_fan_manual(
    app: AppHandle,
    state: State<'_, AppState>,
    args: SetFanManualArgs,
) -> Result<FanControlStatus, IpcError> {
    ensure_admin()?;
    let _operation = state.fan_control.lock_operation().await;
    ensure_control_temperature(&state)?;
    state
        .fan_control
        .set_manual(args.fan_id.clone(), args.pwm)
        .map_err(AppError::Invalid)?;
    if let Err(e) = state.hw_client.set_fan_manual(args.fan_id, args.pwm).await {
        let status = state.fan_control.hold(format!("fan write failed: {e}"));
        let _ = state.hw_client.reset_fans().await;
        let _ = app.emit("fan-control:changed", &status);
        return Err(AppError::Other(format!("set_fan_manual: {e}")).into());
    }
    let status = state.fan_control.status();
    let _ = app.emit("fan-control:changed", &status);
    Ok(status)
}

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SetFanCurveArgs {
    pub fan_id: String,
    pub curve: Vec<FanCurvePoint>,
}

#[tauri::command]
#[specta::specta]
pub async fn set_fan_curve(
    app: AppHandle,
    state: State<'_, AppState>,
    args: SetFanCurveArgs,
) -> Result<FanControlStatus, IpcError> {
    ensure_admin()?;
    let _operation = state.fan_control.lock_operation().await;
    let curve = normalize_curve(args.curve).map_err(AppError::Invalid)?;
    let target_pwm = interpolate_curve(&curve, ensure_control_temperature(&state)?);
    if state.fan_control.status().fuse_hold {
        return Err(AppError::Invalid(
            "fan control is in fuse hold; reset all controls first".into(),
        )
        .into());
    }
    if let Err(e) = state
        .hw_client
        .set_fan_manual(args.fan_id.clone(), target_pwm)
        .await
    {
        return Err(AppError::Other(format!("validate fan curve write: {e}")).into());
    }
    let status = match state.fan_control.set_curve(args.fan_id, curve) {
        Ok(status) => status,
        Err(e) => {
            let _ = state.hw_client.reset_fans().await;
            return Err(AppError::Invalid(e).into());
        }
    };
    let _ = app.emit("fan-control:changed", &status);
    Ok(status)
}

#[derive(serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ResetFanControlArgs {
    pub fan_id: String,
}

#[tauri::command]
#[specta::specta]
pub async fn reset_fan_control(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ResetFanControlArgs,
) -> Result<FanControlStatus, IpcError> {
    let _operation = state.fan_control.lock_operation().await;
    state
        .hw_client
        .reset_fan(args.fan_id.clone())
        .await
        .map_err(|e| AppError::Other(format!("reset_fan_control: {e}")))?;
    let status = state.fan_control.reset_entry(&args.fan_id);
    let _ = app.emit("fan-control:changed", &status);
    Ok(status)
}

#[tauri::command]
#[specta::specta]
pub async fn reset_all_fan_controls(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FanControlStatus, IpcError> {
    let _operation = state.fan_control.lock_operation().await;
    state
        .hw_client
        .reset_fans()
        .await
        .map_err(|e| AppError::Other(format!("reset_all_fan_controls: {e}")))?;
    let status = state.fan_control.clear_all();
    let _ = app.emit("fan-control:changed", &status);
    Ok(status)
}

fn ensure_admin() -> Result<(), IpcError> {
    if admin::is_elevated() {
        Ok(())
    } else {
        Err(AppError::Invalid("fan control requires administrator privileges".into()).into())
    }
}

fn ensure_control_temperature(state: &AppState) -> Result<f64, IpcError> {
    let temperature = state
        .last_hw_snapshot
        .read()
        .as_ref()
        .and_then(fresh_control_temperature);
    match temperature {
        Some(t) if t < 90.0 && state.hw_client.status() == HelperStatus::Running => Ok(t),
        _ => Err(AppError::Invalid(
            "fresh, safe CPU temperature is required for fan control".into(),
        )
        .into()),
    }
}
