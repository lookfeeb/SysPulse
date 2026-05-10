use crate::error::AppError;
use crate::storage::store::DbPool;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct TrafficDelta {
    pub luid: u64,
    pub date_local: NaiveDate,
    pub bytes_recv: u64,
    pub bytes_sent: u64,
}

pub enum WriterMsg {
    Delta(TrafficDelta),
    Flush(tokio::sync::oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct WriterHandle {
    tx: mpsc::Sender<WriterMsg>,
}

impl WriterHandle {
    pub fn try_send_delta(&self, d: TrafficDelta) {
        let _ = self.tx.try_send(WriterMsg::Delta(d));
    }

    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(WriterMsg::Flush(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

pub fn spawn_writer(pool: Arc<DbPool>) -> WriterHandle {
    let (tx, mut rx) = mpsc::channel::<WriterMsg>(1024);
    tauri::async_runtime::spawn(async move {
        let mut buffer: HashMap<(String, u64), (u64, u64)> = HashMap::new();
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = flush(&pool, &mut buffer).await {
                        tracing::warn!(?e, "db writer flush failed");
                    }
                }
                msg = rx.recv() => match msg {
                    None => break,
                    Some(WriterMsg::Delta(d)) => {
                        if d.bytes_recv == 0 && d.bytes_sent == 0 {
                            continue;
                        }
                        let key = (d.date_local.format("%Y-%m-%d").to_string(), d.luid);
                        let entry = buffer.entry(key).or_insert((0, 0));
                        entry.0 = entry.0.saturating_add(d.bytes_recv);
                        entry.1 = entry.1.saturating_add(d.bytes_sent);
                        if buffer.len() > 256 {
                            if let Err(e) = flush(&pool, &mut buffer).await {
                                tracing::warn!(?e, "db writer overflow flush failed");
                            }
                        }
                    }
                    Some(WriterMsg::Flush(reply)) => {
                        if let Err(e) = flush(&pool, &mut buffer).await {
                            tracing::warn!(?e, "db writer manual flush failed");
                        }
                        let _ = reply.send(());
                    }
                }
            }
        }
        let _ = flush(&pool, &mut buffer).await;
    });
    WriterHandle { tx }
}

async fn flush(
    pool: &Arc<DbPool>,
    buf: &mut HashMap<(String, u64), (u64, u64)>,
) -> crate::error::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let pool = pool.clone();
    let entries: Vec<_> = buf.drain().collect();
    tokio::task::spawn_blocking(move || -> crate::error::Result<()> {
        let mut conn = pool.get().map_err(AppError::DbPool)?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO traffic_daily(date_iso, luid, bytes_recv, bytes_sent)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(date_iso, luid) DO UPDATE SET
                   bytes_recv = bytes_recv + excluded.bytes_recv,
                   bytes_sent = bytes_sent + excluded.bytes_sent",
            )?;
            for ((date, luid), (recv, sent)) in entries {
                stmt.execute(rusqlite::params![
                    date,
                    luid as i64,
                    recv as i64,
                    sent as i64,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Other(format!("join: {e}")))??;
    Ok(())
}
