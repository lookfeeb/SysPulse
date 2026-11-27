//! Typed calls on a dedicated in-process .NET worker. Bounded queue and deadlines
//! keep driver waits off Tokio/UI threads. Expired queued control writes are skipped.
use super::{binary, native::NativeHardware, snapshot::HwSnapshot};
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum HelperStatus {
    Starting,
    Running,
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
    #[error("hardware worker queue is full")]
    Busy,
    #[error("hardware module not running")]
    NotRunning,
    #[error("hardware call timed out")]
    Timeout,
    #[error("hardware module: {0}")]
    Module(String),
}
enum Operation {
    Snapshot,
    SetFan(String, f64),
    ResetFan(String),
    ResetFans,
    Stop,
}
enum Output {
    Snapshot(HwSnapshot),
    Unit,
}
struct Job {
    op: Operation,
    deadline: Instant,
    reply: oneshot::Sender<Result<Output, HwClientError>>,
}
impl Job {
    fn is_expired(&self) -> bool {
        self.reply.is_closed() || Instant::now() >= self.deadline
    }
}
struct Inner {
    path: PathBuf,
    tx: Mutex<Option<mpsc::Sender<Job>>>,
    started: AtomicBool,
    stopping: Arc<AtomicBool>,
    status: watch::Sender<HelperStatus>,
    events: broadcast::Sender<HelperStatusEvent>,
    done: watch::Sender<bool>,
}
#[derive(Clone)]
pub struct HwClient {
    inner: Arc<Inner>,
}
impl HwClient {
    pub fn new(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                tx: Mutex::new(None),
                started: AtomicBool::new(false),
                stopping: Arc::new(AtomicBool::new(false)),
                status: watch::channel(HelperStatus::Starting).0,
                events: broadcast::channel(8).0,
                done: watch::channel(false).0,
            }),
        }
    }
    pub fn status(&self) -> HelperStatus {
        *self.inner.status.borrow()
    }
    pub(crate) fn watch_status(&self) -> watch::Receiver<HelperStatus> {
        self.inner.status.subscribe()
    }
    pub fn subscribe_status(&self) -> broadcast::Receiver<HelperStatusEvent> {
        self.inner.events.subscribe()
    }
    pub fn start(&self) {
        if self.inner.stopping.load(Ordering::Acquire)
            || self.inner.started.swap(true, Ordering::AcqRel)
        {
            return;
        }
        let (tx, rx) = mpsc::channel(16);
        *self.inner.tx.lock() = Some(tx);
        let state = Arc::downgrade(&self.inner);
        let path = self.inner.path.clone();
        let stop = self.inner.stopping.clone();
        let done = self.inner.done.clone();
        // A native driver hang cannot be forcibly aborted safely in-process.
        // Never start a replacement collector over the same live hardware.
        if let Err(error) = std::thread::Builder::new()
            .name("hardware-bridge".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker(&state, path, rx, stop)
                }));
                if let Some(inner) = state.upgrade() {
                    if result.is_err() || *inner.status.borrow() != HelperStatus::Unavailable {
                        set_status(
                            &inner,
                            HelperStatus::Unavailable,
                            Some(
                                if result.is_err() {
                                    "hardware worker panicked"
                                } else {
                                    "hardware worker stopped"
                                }
                                .into(),
                            ),
                        );
                    }
                    inner.tx.lock().take();
                }
                done.send_replace(true);
            })
        {
            self.inner.tx.lock().take();
            set_status(
                &self.inner,
                HelperStatus::Unavailable,
                Some(error.to_string()),
            );
            self.inner.done.send_replace(true);
        }
        let state = Arc::downgrade(&self.inner);
        let mut status = self.watch_status();
        tauri::async_runtime::spawn(async move {
            let ready = async {
                while *status.borrow_and_update() == HelperStatus::Starting {
                    if status.changed().await.is_err() {
                        break;
                    }
                }
            };
            if tokio::time::timeout(Duration::from_secs(30), ready)
                .await
                .is_err()
            {
                if let Some(inner) = state.upgrade() {
                    inner.stopping.store(true, Ordering::Release);
                    set_status(
                        &inner,
                        HelperStatus::Unavailable,
                        Some(
                            "hardware initialization exceeded 30 seconds; restart the application"
                                .into(),
                        ),
                    );
                    if let Some(tx) = inner.tx.lock().as_ref() {
                        let (reply, _) = oneshot::channel();
                        let _ = tx.try_send(Job {
                            op: Operation::Stop,
                            deadline: Instant::now(),
                            reply,
                        });
                    }
                }
            }
        });
    }
    async fn request(&self, op: Operation) -> Result<Output, HwClientError> {
        if self.status() != HelperStatus::Running || self.inner.stopping.load(Ordering::Acquire) {
            return Err(HwClientError::NotRunning);
        }
        let sender = self
            .inner
            .tx
            .lock()
            .clone()
            .ok_or(HwClientError::NotRunning)?;
        let (reply, result) = oneshot::channel();
        sender
            .try_send(Job {
                op,
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => HwClientError::Busy,
                _ => HwClientError::NotRunning,
            })?;
        tokio::time::timeout(REQUEST_TIMEOUT, result)
            .await
            .map_err(|_| HwClientError::Timeout)?
            .map_err(|_| HwClientError::NotRunning)?
    }
    pub async fn snapshot(&self) -> Result<HwSnapshot, HwClientError> {
        match self.request(Operation::Snapshot).await? {
            Output::Snapshot(s) => Ok(s),
            _ => Err(HwClientError::Module("Invalid snapshot result".into())),
        }
    }
    pub async fn set_fan_manual(&self, fan_id: String, pwm: f64) -> Result<(), HwClientError> {
        if !pwm.is_finite() || !(0.0..=100.0).contains(&pwm) {
            return Err(HwClientError::Module("Invalid fan PWM".into()));
        }
        self.request(Operation::SetFan(fan_id, pwm)).await?;
        Ok(())
    }
    pub async fn reset_fan(&self, fan_id: String) -> Result<(), HwClientError> {
        self.request(Operation::ResetFan(fan_id)).await?;
        Ok(())
    }
    pub async fn reset_fans(&self) -> Result<(), HwClientError> {
        self.request(Operation::ResetFans).await?;
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::Release);
        set_status(
            &self.inner,
            HelperStatus::Unavailable,
            Some("shutdown".into()),
        );
        if !self.inner.started.load(Ordering::Acquire) {
            return;
        }
        if let Some(sender) = self.inner.tx.lock().as_ref() {
            let (reply, _) = oneshot::channel();
            let _ = sender.try_send(Job {
                op: Operation::Stop,
                deadline: Instant::now(),
                reply,
            });
        }
        let mut done = self.inner.done.subscribe();
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}
fn set_status(inner: &Inner, status: HelperStatus, reason: Option<String>) {
    inner.status.send_replace(status);
    let _ = inner.events.send(HelperStatusEvent { status, reason });
}
fn worker(state: &Weak<Inner>, path: PathBuf, mut rx: mpsc::Receiver<Job>, stop: Arc<AtomicBool>) {
    let mut native = match NativeHardware::load(&path) {
        Ok(native) => native,
        Err(error) => {
            if let Some(inner) = state.upgrade() {
                set_status(&inner, HelperStatus::Unavailable, Some(error.clone()));
            }
            tracing::error!("in-process hardware load failed: {error}");
            return;
        }
    };
    if let Err(error) = native.invoke(0, "", 0.0) {
        if let Some(inner) = state.upgrade() {
            set_status(&inner, HelperStatus::Unavailable, Some(error.clone()));
        }
        tracing::error!("hardware initialization failed: {error}");
        return;
    }
    if !stop.load(Ordering::Acquire) {
        if let Some(inner) = state.upgrade() {
            set_status(&inner, HelperStatus::Running, None);
        }
    }
    while !stop.load(Ordering::Acquire) {
        let Some(job) = rx.blocking_recv() else {
            break;
        };
        if stop.load(Ordering::Acquire) || matches!(job.op, Operation::Stop) {
            break;
        }
        if job.is_expired() {
            continue;
        }
        let changes_control = matches!(
            job.op,
            Operation::SetFan(..) | Operation::ResetFan(..) | Operation::ResetFans
        );
        let result = match job.op {
            Operation::Snapshot => native
                .invoke(1, "", 0.0)
                .and_then(binary::decode)
                .map(Output::Snapshot),
            Operation::SetFan(id, value) => native.invoke(2, &id, value).map(|_| Output::Unit),
            Operation::ResetFan(id) => native.invoke(3, &id, 0.0).map(|_| Output::Unit),
            Operation::ResetFans => native.invoke(4, "", 0.0).map(|_| Output::Unit),
            Operation::Stop => unreachable!(),
        }
        .map_err(HwClientError::Module);
        if job.reply.send(result).is_err() && changes_control {
            // Cancellation cannot interrupt an in-flight driver call. If a
            // timed-out write eventually completes, restore firmware control.
            if let Err(error) = native.invoke(4, "", 0.0) {
                tracing::error!("late hardware control restoration failed: {error}");
            }
        }
    }
    if let Err(error) = native.invoke(5, "", 0.0) {
        tracing::warn!("hardware shutdown failed: {error}");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canceled_and_expired_control_jobs_are_not_executed() {
        let (reply, receiver) = oneshot::channel();
        let mut job = Job {
            op: Operation::ResetFans,
            deadline: Instant::now() + Duration::from_secs(5),
            reply,
        };
        assert!(!job.is_expired());
        drop(receiver);
        assert!(job.is_expired());
        let (reply, _receiver) = oneshot::channel();
        job.reply = reply;
        job.deadline = Instant::now() - Duration::from_millis(1);
        assert!(job.is_expired());
    }
    #[tokio::test]
    async fn closed_module_and_invalid_pwm_do_not_queue_controls() {
        let client = HwClient::new(PathBuf::new());
        assert!(matches!(
            client.snapshot().await,
            Err(HwClientError::NotRunning)
        ));
        assert!(client.set_fan_manual("fan".into(), f64::NAN).await.is_err());
        client.shutdown().await;
        client.start(); // A late start after shutdown must not create a worker or wait forever.
        client.shutdown().await;
        assert_eq!(client.status(), HelperStatus::Unavailable);
    }
    #[tokio::test]
    #[ignore = "real private runtime, hardware reads and application call path; no fan writes"]
    async fn real_worker_snapshot_and_shutdown() {
        let client = HwClient::new(std::env::var("HW_NATIVE_DIR").unwrap().into());
        let mut status = client.watch_status();
        client.start();
        tokio::time::timeout(Duration::from_secs(35), async {
            while *status.borrow_and_update() == HelperStatus::Starting {
                status.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(client.status(), HelperStatus::Running);
        tokio::time::sleep(Duration::from_secs(6)).await;
        let mut times = Vec::new();
        for _ in 0..500 {
            let start = Instant::now();
            let sample = client.snapshot().await.unwrap();
            assert!(sample.memory.as_ref().unwrap().total_bytes > 0);
            assert!(!sample.motherboard.as_ref().unwrap().interfaces.is_empty());
            times.push(start.elapsed().as_nanos());
        }
        times.sort();
        println!(
            "worker + ABI + decoding: p50={}ns p95={}ns",
            times[250], times[475]
        );
        if let Ok(path) = std::env::var("HW_NATIVE_JSON") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&client.snapshot().await.unwrap()).unwrap(),
            )
            .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(10), client.shutdown())
            .await
            .unwrap();
        assert!(matches!(
            client.snapshot().await,
            Err(HwClientError::NotRunning)
        ));
    }
}
