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
    Flush(tokio::sync::oneshot::Sender<crate::error::Result<()>>),
}

#[derive(Clone)]
pub struct WriterHandle {
    tx: mpsc::Sender<WriterMsg>,
}

impl WriterHandle {
    pub async fn send_delta(&self, d: TrafficDelta) {
        if let Err(e) = self.tx.send(WriterMsg::Delta(d)).await {
            tracing::warn!(?e, "db writer delta dropped because writer is unavailable");
        }
    }

    pub async fn flush(&self) -> crate::error::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(WriterMsg::Flush(tx))
            .await
            .map_err(|_| AppError::Other("流量写入任务已停止".into()))?;
        rx.await
            .map_err(|_| AppError::Other("流量写入任务未返回结果".into()))?
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
                        let result = flush(&pool, &mut buffer).await;
                        if let Err(e) = &result {
                            tracing::warn!(?e, "db writer manual flush failed");
                        }
                        let _ = reply.send(result);
                    }
                }
            }
        }
        if let Err(error) = flush(&pool, &mut buffer).await {
            tracing::error!(?error, pending = buffer.len(), "final traffic flush failed");
        }
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
    // 单一 writer 在等待提交时不会修改缓冲；失败（包括任务失败）保留原批次。
    let entries: Vec<_> = buf
        .iter()
        .map(|(key, value)| (key.clone(), *value))
        .collect();
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
    buf.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_transaction_retains_deltas_and_retry_commits_once() {
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(r2d2_sqlite::SqliteConnectionManager::memory())
                .unwrap(),
        );
        pool.get().unwrap().execute_batch(
            "CREATE TABLE traffic_daily(date_iso TEXT, luid INTEGER, bytes_recv INTEGER, bytes_sent INTEGER, PRIMARY KEY(date_iso,luid));
             CREATE TRIGGER reject_write BEFORE INSERT ON traffic_daily BEGIN SELECT RAISE(ABORT, 'busy test'); END;"
        ).unwrap();
        let mut buffer = HashMap::from([
            (("2026-09-28".into(), 1), (100, 200)),
            (("2026-09-28".into(), 2), (300, 400)),
        ]);
        let expected = buffer.clone();
        assert!(flush(&pool, &mut buffer).await.is_err());
        assert_eq!(buffer, expected);
        pool.get()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_write;")
            .unwrap();
        flush(&pool, &mut buffer).await.unwrap();
        assert!(buffer.is_empty());
        flush(&pool, &mut buffer).await.unwrap();
        let totals: (i64, i64) = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT SUM(bytes_recv), SUM(bytes_sent) FROM traffic_daily",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(totals, (400, 600));
    }

    #[tokio::test]
    async fn manual_flush_reports_writer_failure() {
        let (tx, mut rx) = mpsc::channel(1);
        let handle = WriterHandle { tx };
        let reply = tokio::spawn(async move {
            if let Some(WriterMsg::Flush(reply)) = rx.recv().await {
                let _ = reply.send(Err(AppError::Other("write failed".into())));
            }
        });
        assert!(handle
            .flush()
            .await
            .unwrap_err()
            .to_string()
            .contains("write failed"));
        reply.await.unwrap();
        assert!(handle.flush().await.is_err());
    }
}
