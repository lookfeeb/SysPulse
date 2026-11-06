use crate::error::AppError;
use crate::storage::queries::{query_history, DailyTraffic, HistoryGranularity, HistoryQuery};
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
    Query(
        HistoryQuery,
        tokio::sync::oneshot::Sender<crate::error::Result<Vec<DailyTraffic>>>,
    ),
}

#[derive(Clone)]
pub struct WriterHandle {
    tx: mpsc::Sender<WriterMsg>,
}

impl WriterHandle {
    pub async fn query_history(
        &self,
        query: HistoryQuery,
    ) -> crate::error::Result<Vec<DailyTraffic>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(WriterMsg::Query(query, tx))
            .await
            .map_err(|_| AppError::Other("流量写入任务已停止".into()))?;
        rx.await
            .map_err(|_| AppError::Other("流量查询任务未返回结果".into()))?
    }

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
                    Some(WriterMsg::Query(query, reply)) => {
                        // Serialize reads with flushes so buffered bytes appear exactly once.
                        // Reading history must not force a disk write on every UI refresh.
                        let pool = pool.clone();
                        let pending = buffer.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            query_with_pending(&pool, &query, &pending)
                        }).await.map_err(|e| AppError::Other(format!("join: {e}")))
                            .and_then(|result| result);
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

fn query_with_pending(
    pool: &DbPool,
    query: &HistoryQuery,
    pending: &HashMap<(String, u64), (u64, u64)>,
) -> crate::error::Result<Vec<DailyTraffic>> {
    let rows = query_history(pool, query)?;
    let (from, to) = query.date_bounds()?;
    // query_history has already validated the interface and date inputs.
    let iface = query
        .iface
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());
    let mut rows: std::collections::BTreeMap<_, _> = rows
        .into_iter()
        .map(|row| (row.date.clone(), row))
        .collect();
    for ((date, luid), (recv, sent)) in pending {
        if from.as_ref().is_some_and(|from| date < from)
            || to.as_ref().is_some_and(|to| date > to)
            || iface.is_some_and(|id| id != *luid as i64)
        {
            continue;
        }
        let key = match query.granularity {
            HistoryGranularity::Day => date.clone(),
            HistoryGranularity::Month => date[..7].to_string(),
        };
        let row = rows.entry(key.clone()).or_insert_with(|| DailyTraffic {
            date: key,
            iface: iface.map(|id| id.to_string()),
            bytes_recv: 0,
            bytes_sent: 0,
        });
        row.bytes_recv = row
            .bytes_recv
            .saturating_add((*recv).min(i64::MAX as u64) as i64);
        row.bytes_sent = row
            .bytes_sent
            .saturating_add((*sent).min(i64::MAX as u64) as i64);
    }
    Ok(rows.into_values().collect())
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
    async fn realtime_history_includes_pending_bytes_once_before_and_after_flush() {
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(r2d2_sqlite::SqliteConnectionManager::memory())
                .unwrap(),
        );
        pool.get().unwrap().execute_batch(
            "CREATE TABLE traffic_daily(date_iso TEXT, luid INTEGER, bytes_recv INTEGER, bytes_sent INTEGER, PRIMARY KEY(date_iso,luid));
             INSERT INTO traffic_daily VALUES ('2026-10-31', 1, 100, 200), ('2026-11-01', 1, 300, 400);"
        ).unwrap();
        let mut pending = HashMap::from([
            (("2026-10-31".into(), 1), (10, 20)),
            (("2026-11-01".into(), 1), (30, 40)),
            (("2026-11-01".into(), 2), (50, 60)),
            (("2026-11-02".into(), 2), (70, 80)),
            (("2026-12-01".into(), 1), (900, 900)),
        ]);
        let query = HistoryQuery {
            from: Some("2026-10-31".into()),
            to: Some("2026-11-02".into()),
            granularity: HistoryGranularity::Day,
            iface: None,
        };
        let values = |rows: Vec<DailyTraffic>| {
            rows.into_iter()
                .map(|row| (row.date, row.bytes_recv, row.bytes_sent))
                .collect::<Vec<_>>()
        };
        let expected = vec![
            ("2026-10-31".into(), 110, 220),
            ("2026-11-01".into(), 380, 500),
            ("2026-11-02".into(), 70, 80),
        ];
        assert_eq!(
            values(query_with_pending(&pool, &query, &pending).unwrap()),
            expected
        );
        assert_eq!(
            values(query_with_pending(&pool, &query, &pending).unwrap()),
            expected
        );
        // Reads leave the database untouched.
        assert_eq!(values(query_history(&pool, &query).unwrap())[0].1, 100);
        let monthly = HistoryQuery {
            granularity: HistoryGranularity::Month,
            ..query.clone()
        };
        assert_eq!(
            values(query_with_pending(&pool, &monthly, &pending).unwrap()),
            vec![("2026-10".into(), 110, 220), ("2026-11".into(), 450, 580)]
        );
        let iface = HistoryQuery {
            iface: Some("2".into()),
            ..monthly
        };
        assert_eq!(
            values(query_with_pending(&pool, &iface, &pending).unwrap()),
            vec![("2026-11".into(), 120, 140)]
        );
        let all = HistoryQuery {
            from: None,
            to: None,
            ..query.clone()
        };
        let all_before_flush = values(query_with_pending(&pool, &all, &pending).unwrap());
        assert_eq!(all_before_flush.len(), 4);
        assert_eq!(
            all_before_flush.last().unwrap(),
            &("2026-12-01".into(), 900, 900)
        );
        let all_months = HistoryQuery {
            granularity: HistoryGranularity::Month,
            ..all.clone()
        };
        assert_eq!(
            query_with_pending(&pool, &all_months, &pending)
                .unwrap()
                .len(),
            3
        );
        let from_november = HistoryQuery {
            from: Some("2026-11-01".into()),
            ..all.clone()
        };
        assert_eq!(
            query_with_pending(&pool, &from_november, &pending)
                .unwrap()
                .len(),
            3
        );
        flush(&pool, &mut pending).await.unwrap();
        assert_eq!(
            values(query_with_pending(&pool, &all, &pending).unwrap()),
            all_before_flush
        );
        assert_eq!(
            values(query_with_pending(&pool, &query, &pending).unwrap()),
            expected
        );
    }

    #[tokio::test]
    async fn writer_query_observes_queued_deltas_and_reports_invalid_queries() {
        let pool = Arc::new(
            r2d2::Pool::builder()
                .max_size(1)
                .build(r2d2_sqlite::SqliteConnectionManager::memory())
                .unwrap(),
        );
        pool.get().unwrap().execute_batch(
            "CREATE TABLE traffic_daily(date_iso TEXT, luid INTEGER, bytes_recv INTEGER, bytes_sent INTEGER, PRIMARY KEY(date_iso,luid));"
        ).unwrap();
        let writer = spawn_writer(pool);
        let query = HistoryQuery {
            from: Some("2026-11-06".into()),
            to: Some("2026-11-06".into()),
            granularity: HistoryGranularity::Day,
            iface: None,
        };
        writer
            .send_delta(TrafficDelta {
                luid: 1,
                date_local: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
                bytes_recv: 100,
                bytes_sent: 200,
            })
            .await;
        for _ in 0..3 {
            let rows = writer.query_history(query.clone()).await.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!((rows[0].bytes_recv, rows[0].bytes_sent), (100, 200));
            writer.flush().await.unwrap();
        }
        assert!(writer
            .query_history(HistoryQuery {
                from: Some("invalid".into()),
                ..query
            })
            .await
            .is_err());
    }

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
