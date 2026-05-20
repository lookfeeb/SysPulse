use crate::error::{AppError, IpcError};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use winreg::enums::*;
use winreg::RegKey;

/// Prevent child console windows from flashing on screen.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Legacy registry key path previously used for current-user auto-start.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Legacy value name in the registry.
const VALUE_NAME: &str = "SysPulse";
/// Windows Task Scheduler task used for reliable elevated logon startup.
const TASK_NAME: &str = "SysPulse";

// ============================================================================
// IPC commands — auto-start via Windows Task Scheduler.
//
// Release builds request administrator privileges. HKCU\...\Run is not a
// reliable elevated startup mechanism, and unquoted paths under "Program Files"
// are ambiguous. A per-user scheduled task with RL HIGHEST starts the app at
// logon without a UAC prompt after the user has enabled it once.
// ============================================================================

#[tauri::command]
#[specta::specta]
pub fn autostart_is_enabled() -> Result<bool, IpcError> {
    let exe_path = current_exe_path()?;
    let enabled = scheduled_task_points_to(&exe_path)?;
    if !enabled {
        delete_legacy_run_value();
    }
    Ok(enabled)
}

#[tauri::command]
#[specta::specta]
pub fn autostart_enable() -> Result<(), IpcError> {
    let exe_path = current_exe_path()?;
    create_scheduled_task(&exe_path)?;
    delete_legacy_run_value();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn autostart_disable() -> Result<(), IpcError> {
    delete_scheduled_task()?;
    delete_legacy_run_value();
    Ok(())
}

fn current_exe_path() -> Result<PathBuf, IpcError> {
    std::env::current_exe()
        .map_err(|e| AppError::Config(format!("failed to get exe path: {e}")).into())
}

fn create_scheduled_task(exe_path: &Path) -> Result<(), IpcError> {
    let task_command = quote_for_task_action(exe_path);
    let output = Command::new("schtasks")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "/Create",
            "/TN",
            TASK_NAME,
            "/TR",
            &task_command,
            "/SC",
            "ONLOGON",
            "/RL",
            "HIGHEST",
            "/F",
        ])
        .output()
        .map_err(|e| AppError::Config(format!("failed to run schtasks: {e}")))?;

    if output.status.success() {
        return Ok(());
    }

    Err(AppError::Config(format!(
        "failed to create startup task: {}",
        command_output_message(&output)
    ))
    .into())
}

fn delete_scheduled_task() -> Result<(), IpcError> {
    if !scheduled_task_exists()? {
        return Ok(());
    }

    let output = Command::new("schtasks")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output()
        .map_err(|e| AppError::Config(format!("failed to run schtasks: {e}")))?;

    if output.status.success() {
        return Ok(());
    }

    Err(AppError::Config(format!(
        "failed to delete startup task: {}",
        command_output_message(&output)
    ))
    .into())
}

fn scheduled_task_exists() -> Result<bool, IpcError> {
    let output = Command::new("schtasks")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/Query", "/TN", TASK_NAME])
        .output()
        .map_err(|e| AppError::Config(format!("failed to run schtasks: {e}")))?;
    Ok(output.status.success())
}

fn scheduled_task_points_to(exe_path: &Path) -> Result<bool, IpcError> {
    let output = Command::new("schtasks")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/Query", "/TN", TASK_NAME, "/XML"])
        .output()
        .map_err(|e| AppError::Config(format!("failed to run schtasks: {e}")))?;

    if !output.status.success() {
        return Ok(false);
    }

    let xml = decode_process_bytes(&output.stdout);
    let Some(command) = extract_xml_tag(&xml, "Command") else {
        return Ok(false);
    };

    Ok(paths_equal(Path::new(command.trim_matches('"')), exe_path))
}

fn delete_legacy_run_value() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(run_key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_WRITE) {
        let _ = run_key.delete_value(VALUE_NAME);
    }
}

fn quote_for_task_action(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = xml.find(&start_tag)? + start_tag.len();
    let end = xml[start..].find(&end_tag)? + start;
    Some(unescape_xml_basic(&xml[start..end]))
}

fn unescape_xml_basic(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn command_output_message(output: &std::process::Output) -> String {
    let stderr = decode_process_bytes(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    decode_process_bytes(&output.stdout).trim().to_string()
}

fn decode_process_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&words);
    }

    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_task_action_path() {
        assert_eq!(
            quote_for_task_action(Path::new(r"D:\Program Files\SysPulse\syspulse.exe")),
            r#""D:\Program Files\SysPulse\syspulse.exe""#
        );
    }

    #[test]
    fn extracts_escaped_command_from_task_xml() {
        let xml = r#"<Task><Actions><Exec><Command>D:\A&amp;B\syspulse.exe</Command></Exec></Actions></Task>"#;
        assert_eq!(
            extract_xml_tag(xml, "Command").as_deref(),
            Some(r"D:\A&B\syspulse.exe")
        );
    }
}
