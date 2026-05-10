use crate::error::{AppError, Result};
use crate::storage::store::DbPool;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum HistoryGranularity {
    Day,
    Month,
}

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub from: String,
    pub to: String,
    #[serde(default = "default_granularity")]
    pub granularity: HistoryGranularity,
    pub iface: Option<String>,
}

fn default_granularity() -> HistoryGranularity {
    HistoryGranularity::Day
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DailyTraffic {
    pub date: String,
    pub iface: Option<String>,
    pub bytes_recv: i64,
    pub bytes_sent: i64,
}

pub fn query_history(pool: &DbPool, q: &HistoryQuery) -> Result<Vec<DailyTraffic>> {
    NaiveDate::parse_from_str(&q.from, "%Y-%m-%d")
        .map_err(|e| AppError::Invalid(format!("from date: {e}")))?;
    NaiveDate::parse_from_str(&q.to, "%Y-%m-%d")
        .map_err(|e| AppError::Invalid(format!("to date: {e}")))?;

    let conn = pool.get().map_err(AppError::DbPool)?;

    let rows = match (q.granularity, q.iface.as_deref()) {
        (HistoryGranularity::Day, None) => fetch_day_all(&conn, &q.from, &q.to)?,
        (HistoryGranularity::Day, Some(luid)) => {
            let luid: i64 = luid
                .parse()
                .map_err(|e| AppError::Invalid(format!("iface luid: {e}")))?;
            fetch_day_one(&conn, luid, &q.from, &q.to)?
        }
        (HistoryGranularity::Month, None) => fetch_month_all(&conn, &q.from, &q.to)?,
        (HistoryGranularity::Month, Some(luid)) => {
            let luid: i64 = luid
                .parse()
                .map_err(|e| AppError::Invalid(format!("iface luid: {e}")))?;
            fetch_month_one(&conn, luid, &q.from, &q.to)?
        }
    };
    Ok(rows)
}

fn fetch_day_all(conn: &rusqlite::Connection, from: &str, to: &str) -> Result<Vec<DailyTraffic>> {
    let mut stmt = conn.prepare(
        "SELECT date_iso, SUM(bytes_recv), SUM(bytes_sent)
           FROM traffic_daily
          WHERE date_iso BETWEEN ?1 AND ?2
          GROUP BY date_iso
          ORDER BY date_iso ASC",
    )?;
    let iter = stmt.query_map([from, to], |r| {
        Ok(DailyTraffic {
            date: r.get::<_, String>(0)?,
            iface: None,
            bytes_recv: r.get::<_, i64>(1)?,
            bytes_sent: r.get::<_, i64>(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in iter {
        out.push(row?);
    }
    Ok(out)
}

fn fetch_day_one(
    conn: &rusqlite::Connection,
    luid: i64,
    from: &str,
    to: &str,
) -> Result<Vec<DailyTraffic>> {
    let mut stmt = conn.prepare(
        "SELECT date_iso, bytes_recv, bytes_sent
           FROM traffic_daily
          WHERE luid = ?1 AND date_iso BETWEEN ?2 AND ?3
          ORDER BY date_iso ASC",
    )?;
    let iter = stmt.query_map(rusqlite::params![luid, from, to], |r| {
        Ok(DailyTraffic {
            date: r.get::<_, String>(0)?,
            iface: Some(luid.to_string()),
            bytes_recv: r.get::<_, i64>(1)?,
            bytes_sent: r.get::<_, i64>(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in iter {
        out.push(row?);
    }
    Ok(out)
}

fn fetch_month_all(conn: &rusqlite::Connection, from: &str, to: &str) -> Result<Vec<DailyTraffic>> {
    let mut stmt = conn.prepare(
        "SELECT substr(date_iso,1,7), SUM(bytes_recv), SUM(bytes_sent)
           FROM traffic_daily
          WHERE date_iso BETWEEN ?1 AND ?2
          GROUP BY substr(date_iso,1,7)
          ORDER BY 1 ASC",
    )?;
    let iter = stmt.query_map([from, to], |r| {
        Ok(DailyTraffic {
            date: r.get::<_, String>(0)?,
            iface: None,
            bytes_recv: r.get::<_, i64>(1)?,
            bytes_sent: r.get::<_, i64>(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in iter {
        out.push(row?);
    }
    Ok(out)
}

fn fetch_month_one(
    conn: &rusqlite::Connection,
    luid: i64,
    from: &str,
    to: &str,
) -> Result<Vec<DailyTraffic>> {
    let mut stmt = conn.prepare(
        "SELECT substr(date_iso,1,7), SUM(bytes_recv), SUM(bytes_sent)
           FROM traffic_daily
          WHERE luid = ?1 AND date_iso BETWEEN ?2 AND ?3
          GROUP BY substr(date_iso,1,7)
          ORDER BY 1 ASC",
    )?;
    let iter = stmt.query_map(rusqlite::params![luid, from, to], |r| {
        Ok(DailyTraffic {
            date: r.get::<_, String>(0)?,
            iface: Some(luid.to_string()),
            bytes_recv: r.get::<_, i64>(1)?,
            bytes_sent: r.get::<_, i64>(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in iter {
        out.push(row?);
    }
    Ok(out)
}

pub fn cleanup_old(pool: &DbPool, retain_days: u32) -> Result<()> {
    let cutoff = chrono::Local::now()
        .date_naive()
        .checked_sub_days(chrono::Days::new(retain_days as u64))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    if cutoff.is_empty() {
        return Ok(());
    }
    let conn = pool.get().map_err(AppError::DbPool)?;
    conn.execute(
        "DELETE FROM traffic_daily WHERE date_iso < ?1",
        rusqlite::params![cutoff],
    )?;
    Ok(())
}
