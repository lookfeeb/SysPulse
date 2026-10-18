//! Subprocess wrapper around `hw-helper.exe`.
//!
//! Lifecycle:
//!   - Spawn the helper as a child process with piped stdin/stdout/stderr.
//!   - A reader task pumps stdout, dispatching responses to one-shot channels
//!     keyed by request id, and forwarding events to a broadcast channel.
//!   - A writer task serializes outgoing requests onto stdin.
//!   - A supervisor owns the child handle and restarts the helper on exits or
//!     repeated request timeouts.

use crate::hw::protocol::{HelperError, HelperEvent, HelperRequest, HelperResponse, RequestParams};
use crate::hw::snapshot::HwSnapshot;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio::time::timeout;

const REQ_TIMEOUT: Duration = Duration::from_millis(3000);
const MAX_TIMEOUTS_BEFORE_RESTART: u32 = 5;
const MAX_RESTART_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum HelperStatus {
    Starting,
    Running,
    Restarting,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatusEvent {
    pub status: HelperStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HwClientError {
    #[error("helper request queue is full")]
    Busy,
    #[error("helper not running")]
    NotRunning,
    #[error("request timed out")]
    Timeout,
    #[error("helper returned error [{code}]: {message}")]
    Helper { code: String, message: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

impl From<HelperError> for HwClientError {
    fn from(e: HelperError) -> Self {
        HwClientError::Helper {
            code: e.code,
            message: e.message,
        }
    }
}

struct InnerState {
    /// id -> oneshot sender awaiting a response
    pending: Mutex<HashMap<u64, oneshot::Sender<HelperResponse>>>,
    /// guarded handle to the writer task
    writer_tx: RwLock<Option<mpsc::Sender<HelperRequest>>>,
    /// control channel to the supervisor that owns the child process
    supervisor_tx: Mutex<Option<mpsc::Sender<SupervisorCmd>>>,
    /// auto-incrementing request id
    next_id: AtomicU64,
    /// consecutive timeouts since last successful response
    timeouts: AtomicI64,
    /// current status
    status: Mutex<HelperStatus>,
    /// status broadcast (to forward to webview as "hw:helper-status")
    status_tx: broadcast::Sender<HelperStatusEvent>,
    /// helper executable path
    helper_path: PathBuf,
    io_tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

#[derive(Debug)]
enum SupervisorCmd {
    Restart(String),
    Shutdown,
}

#[derive(Debug)]
enum SupervisorExit {
    Exited,
    Restart(String),
    Shutdown,
}

#[derive(Clone)]
pub struct HwClient {
    inner: Arc<InnerState>,
}

impl HwClient {
    pub fn new(helper_path: PathBuf) -> Self {
        let (status_tx, _) = broadcast::channel(8);
        Self {
            inner: Arc::new(InnerState {
                pending: Mutex::new(HashMap::new()),
                writer_tx: RwLock::new(None),
                supervisor_tx: Mutex::new(None),
                next_id: AtomicU64::new(1),
                timeouts: AtomicI64::new(0),
                status: Mutex::new(HelperStatus::Starting),
                status_tx,
                helper_path,
                io_tasks: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn status(&self) -> HelperStatus {
        *self.inner.status.lock()
    }

    pub fn subscribe_status(&self) -> broadcast::Receiver<HelperStatusEvent> {
        self.inner.status_tx.subscribe()
    }

    /// Start the helper (and the supervisor that auto-restarts it). Idempotent.
    pub fn start(&self) {
        let mut guard = self.inner.supervisor_tx.lock();
        if guard.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<SupervisorCmd>(8);
        *guard = Some(tx);
        drop(guard);

        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            supervisor_loop(inner, rx).await;
        });
    }

    pub async fn snapshot(&self) -> Result<HwSnapshot, HwClientError> {
        let resp = self.request_json("snapshot", None).await?;
        let mut snapshot: HwSnapshot = serde_json::from_value(resp)?;
        snapshot.received_at = Some(std::time::Instant::now());
        Ok(snapshot)
    }

    pub async fn set_fan_manual(&self, fan_id: String, pwm: f64) -> Result<(), HwClientError> {
        self.request_json(
            "set_fan",
            Some(RequestParams {
                fan_id: Some(fan_id),
                mode: Some("manual".into()),
                pwm: Some(pwm),
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn reset_fan(&self, fan_id: String) -> Result<(), HwClientError> {
        self.request_json(
            "reset_fan",
            Some(RequestParams {
                fan_id: Some(fan_id),
                mode: Some("bios".into()),
                pwm: None,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn reset_fans(&self) -> Result<(), HwClientError> {
        let report = self.request_json("reset_fans", None).await?;
        if report
            .get("failed")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0)
        {
            return Err(HwClientError::Helper {
                code: "RESET_FAILED".into(),
                message: "one or more fan controls could not be restored".into(),
            });
        }
        Ok(())
    }

    /// Best-effort graceful shutdown: send "shutdown", give helper 500ms, then
    /// ask the supervisor to terminate the process.
    pub async fn shutdown(&self) {
        let _ = self.request_raw("shutdown", None).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        request_supervisor_shutdown(&self.inner).await;
    }

    async fn request_json(
        &self,
        op: &str,
        params: Option<RequestParams>,
    ) -> Result<Value, HwClientError> {
        let resp = self.request_raw(op, params).await?;
        if resp.ok {
            Ok(resp.data)
        } else {
            Err(resp
                .error
                .map(HwClientError::from)
                .unwrap_or_else(|| HwClientError::Helper {
                    code: "UNKNOWN".into(),
                    message: format!("op={op} failed without error payload"),
                }))
        }
    }

    async fn request_raw(
        &self,
        op: &str,
        params: Option<RequestParams>,
    ) -> Result<HelperResponse, HwClientError> {
        let writer_tx = {
            let guard = self.inner.writer_tx.read().await;
            match &*guard {
                Some(tx) => tx.clone(),
                None => return Err(HwClientError::NotRunning),
            }
        };

        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock();
            if pending.len() >= 32 {
                return Err(HwClientError::Busy);
            }
            pending.insert(id, tx);
        }
        let _pending = PendingRequest {
            state: &self.inner,
            id,
        };

        let req = HelperRequest {
            id,
            op: op.into(),
            params,
        };

        // Include backpressure in the deadline; cancellation also drops the map entry.
        match timeout(REQ_TIMEOUT, async {
            writer_tx.send(req).await.map_err(|_| ())?;
            rx.await.map_err(|_| ())
        })
        .await
        {
            Ok(Ok(resp)) => {
                self.inner.timeouts.store(0, Ordering::Relaxed);
                mark_running_if_available(&self.inner);
                Ok(resp)
            }
            Ok(Err(_recv_err)) => {
                self.inner.pending.lock().remove(&id);
                Err(HwClientError::NotRunning)
            }
            Err(_elapsed) => {
                self.inner.pending.lock().remove(&id);
                let n = self.inner.timeouts.fetch_add(1, Ordering::Relaxed) + 1;
                if should_restart_after_timeouts(n) {
                    let reason = format!("{n} consecutive request timeouts");
                    tracing::warn!("hw helper hit {reason}; requesting restart");
                    request_supervisor_restart(&self.inner, reason);
                }
                Err(HwClientError::Timeout)
            }
        }
    }
}

struct PendingRequest<'a> {
    state: &'a InnerState,
    id: u64,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        self.state.pending.lock().remove(&self.id);
    }
}

impl Drop for HwClient {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            if let Some(tx) = self.inner.supervisor_tx.lock().take() {
                let _ = tx.try_send(SupervisorCmd::Shutdown);
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Supervisor — owns the lifecycle of one helper process. Restarts on failure
// with backoff, gives up after MAX_RESTART_ATTEMPTS.
// ----------------------------------------------------------------------------

async fn supervisor_loop(state: Arc<InnerState>, mut rx: mpsc::Receiver<SupervisorCmd>) {
    let mut attempt: u32 = 0;
    loop {
        set_status(&state, HelperStatus::Starting, None);
        match spawn_one(&state).await {
            Ok(child) => {
                let started = std::time::Instant::now();
                match wait_for_exit_or_command(child, &mut rx).await {
                    SupervisorExit::Exited => {
                        clear_runtime(&state).await;
                        tracing::warn!("hw helper exited; attempting restart");
                        set_status(
                            &state,
                            HelperStatus::Restarting,
                            Some("helper exited".into()),
                        );
                    }
                    SupervisorExit::Restart(reason) => {
                        clear_runtime(&state).await;
                        set_status(&state, HelperStatus::Restarting, Some(reason));
                    }
                    SupervisorExit::Shutdown => {
                        clear_runtime(&state).await;
                        set_status(&state, HelperStatus::Unavailable, Some("shutdown".into()));
                        *state.supervisor_tx.lock() = None;
                        return;
                    }
                }
                // A process that starts then crashes immediately is still a failed attempt.
                if started.elapsed() >= Duration::from_secs(60) {
                    attempt = 0;
                }
            }
            Err(e) => {
                clear_runtime(&state).await;
                tracing::error!("failed to spawn hw helper: {e}");
                set_status(
                    &state,
                    HelperStatus::Restarting,
                    Some(format!("spawn failed: {e}")),
                );
            }
        }

        attempt += 1;
        if attempt >= MAX_RESTART_ATTEMPTS {
            tracing::error!("hw helper restart limit reached; entering Unavailable");
            clear_runtime(&state).await;
            set_status(
                &state,
                HelperStatus::Unavailable,
                Some("restart limit reached".into()),
            );
            *state.supervisor_tx.lock() = None;
            return;
        }

        let backoff = Duration::from_secs(2u64.pow(attempt.min(4)));
        let delay = tokio::time::sleep(backoff);
        tokio::pin!(delay);
        loop {
            tokio::select! {
                _ = &mut delay => { break; }
                cmd = rx.recv() => {
                    match cmd {
                        Some(SupervisorCmd::Shutdown) | None => {
                            set_status(&state, HelperStatus::Unavailable, Some("shutdown".into()));
                            *state.supervisor_tx.lock() = None;
                            return;
                        }
                        Some(SupervisorCmd::Restart(reason)) => {
                            tracing::debug!("coalescing helper restart while backing off: {reason}");
                        }
                    }
                }
            }
        }
    }
}

async fn spawn_one(state: &Arc<InnerState>) -> std::io::Result<Child> {
    if !state.helper_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("helper not found: {}", state.helper_path.display()),
        ));
    }

    let mut cmd = Command::new(&state.helper_path);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn()?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (writer_tx, writer_rx) = mpsc::channel::<HelperRequest>(64);

    clear_pending(state);
    *state.writer_tx.write().await = Some(writer_tx);
    state.timeouts.store(0, Ordering::Relaxed);

    state.io_tasks.lock().extend([
        spawn_writer_task(stdin, writer_rx, state.clone()),
        spawn_stderr_task(stderr),
        spawn_reader_task(stdout, state.clone()),
    ]);

    Ok(child)
}

async fn wait_for_exit_or_command(
    mut child: Child,
    rx: &mut mpsc::Receiver<SupervisorCmd>,
) -> SupervisorExit {
    tokio::select! {
        status = child.wait() => {
            if let Err(e) = status {
                tracing::warn!("hw helper wait failed: {e}");
            }
            SupervisorExit::Exited
        }
        cmd = rx.recv() => {
            match cmd {
                Some(SupervisorCmd::Restart(reason)) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    SupervisorExit::Restart(reason)
                }
                Some(SupervisorCmd::Shutdown) | None => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    SupervisorExit::Shutdown
                }
            }
        }
    }
}

fn spawn_writer_task(
    mut stdin: ChildStdin,
    mut rx: mpsc::Receiver<HelperRequest>,
    state: Arc<InnerState>,
) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            if !state.pending.lock().contains_key(&req.id) {
                continue;
            }
            let json = match serde_json::to_string(&req) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("failed to serialize hw request: {e}");
                    continue;
                }
            };
            if let Err(e) = stdin.write_all(json.as_bytes()).await {
                tracing::warn!("hw helper stdin write failed: {e}");
                request_supervisor_restart(&state, format!("stdin write failed: {e}"));
                break;
            }
            if let Err(e) = stdin.write_all(b"\n").await {
                tracing::warn!("hw helper stdin newline failed: {e}");
                request_supervisor_restart(&state, format!("stdin newline failed: {e}"));
                break;
            }
            if let Err(e) = stdin.flush().await {
                tracing::warn!("hw helper stdin flush failed: {e}");
                request_supervisor_restart(&state, format!("stdin flush failed: {e}"));
                break;
            }
        }
    })
    .abort_handle()
}

fn spawn_stderr_task(stderr: tokio::process::ChildStderr) -> tokio::task::AbortHandle {
    let mut reader = BufReader::new(stderr);
    tokio::spawn(async move {
        while let Ok(Some(line)) = read_bounded_line(&mut reader, 16 * 1024).await {
            tracing::warn!(target: "hw_helper", "{line}");
        }
    })
    .abort_handle()
}

fn spawn_reader_task(
    stdout: tokio::process::ChildStdout,
    state: Arc<InnerState>,
) -> tokio::task::AbortHandle {
    let mut reader = BufReader::new(stdout);
    tokio::spawn(async move {
        while let Ok(Some(line)) = read_bounded_line(&mut reader, 1024 * 1024).await {
            if let Ok(resp) = serde_json::from_str::<HelperResponse>(&line) {
                if resp.id < 0 {
                    tracing::warn!(target: "hw_helper", "bad-id response: {line}");
                    continue;
                }
                let id = resp.id as u64;
                if let Some(sender) = state.pending.lock().remove(&id) {
                    mark_running_if_available(&state);
                    let _ = sender.send(resp);
                } else {
                    // Late responses must not erase repeated deadline failures.
                    tracing::debug!(target: "hw_helper", "stray response id={id} (helper alive, was slow)");
                }
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<HelperEvent>(&line) {
                tracing::info!(target: "hw_helper", event = ev.event, ?ev.data, "event");
                if ev.event == "ready" {
                    mark_running_if_available(&state);
                }
                continue;
            }
            tracing::warn!(target: "hw_helper", "unparseable line: {line}");
        }
        request_supervisor_restart(&state, "helper stdout closed or exceeded protocol limit".into());
    }).abort_handle()
}

async fn clear_runtime(state: &Arc<InnerState>) {
    for task in state.io_tasks.lock().drain(..) {
        task.abort();
    }
    *state.writer_tx.write().await = None;
    clear_pending(state);
    state.timeouts.store(0, Ordering::Relaxed);
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "incomplete helper frame",
            ));
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let count = newline.map_or(available.len(), |p| p + 1);
        if count > limit.saturating_sub(bytes.len()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "helper frame too large",
            ));
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
        }
    }
}

fn clear_pending(state: &Arc<InnerState>) {
    let drained: Vec<oneshot::Sender<HelperResponse>> =
        state.pending.lock().drain().map(|(_, tx)| tx).collect();
    drop(drained);
}

fn should_restart_after_timeouts(timeouts: i64) -> bool {
    timeouts == i64::from(MAX_TIMEOUTS_BEFORE_RESTART)
}

fn request_supervisor_restart(state: &Arc<InnerState>, reason: String) {
    let tx = state.supervisor_tx.lock().as_ref().cloned();
    if let Some(tx) = tx {
        if let Err(e) = tx.try_send(SupervisorCmd::Restart(reason)) {
            tracing::warn!("failed to request hw helper restart: {e}");
        }
    }
}

async fn request_supervisor_shutdown(state: &Arc<InnerState>) {
    let tx = state.supervisor_tx.lock().as_ref().cloned();
    if let Some(tx) = tx {
        let _ = tx.send(SupervisorCmd::Shutdown).await;
    }
}

fn set_status(state: &Arc<InnerState>, new: HelperStatus, reason: Option<String>) {
    *state.status.lock() = new;
    let _ = state.status_tx.send(HelperStatusEvent {
        status: new,
        reason,
    });
}

fn mark_running_if_available(state: &Arc<InnerState>) {
    let mut status = state.status.lock();
    if *status == HelperStatus::Running {
        return;
    }
    *status = HelperStatus::Running;
    drop(status);
    let _ = state.status_tx.send(HelperStatusEvent {
        status: HelperStatus::Running,
        reason: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn protocol_frames_are_bounded_and_must_be_complete() {
        let mut valid = BufReader::with_capacity(2, &b"one\ntwo\n"[..]);
        assert_eq!(
            read_bounded_line(&mut valid, 4).await.unwrap().as_deref(),
            Some("one\n")
        );
        assert_eq!(
            read_bounded_line(&mut valid, 4).await.unwrap().as_deref(),
            Some("two\n")
        );
        assert!(read_bounded_line(&mut valid, 4).await.unwrap().is_none());
        let mut large = BufReader::with_capacity(2, &b"12345\n"[..]);
        assert!(read_bounded_line(&mut large, 4).await.is_err());
        let mut partial = BufReader::new(&b"123"[..]);
        assert!(read_bounded_line(&mut partial, 4).await.is_err());
    }

    #[tokio::test]
    async fn cancelled_queued_requests_release_pending_entries() {
        use futures_util::FutureExt;
        let client = HwClient::new(PathBuf::new());
        let (tx, _rx) = mpsc::channel(1);
        tx.send(HelperRequest {
            id: 0,
            op: "occupied".into(),
            params: None,
        })
        .await
        .unwrap();
        *client.inner.writer_tx.write().await = Some(tx);
        let mut request = Box::pin(client.request_raw("snapshot", None));
        assert!(request.as_mut().now_or_never().is_none());
        assert_eq!(client.inner.pending.lock().len(), 1);
        drop(request);
        assert!(client.inner.pending.lock().is_empty());
    }

    #[test]
    fn restart_threshold_uses_direct_timeout_count() {
        assert!(!should_restart_after_timeouts(0));
        assert!(!should_restart_after_timeouts(4));
        assert!(should_restart_after_timeouts(5));
        assert!(!should_restart_after_timeouts(6));
    }
}
