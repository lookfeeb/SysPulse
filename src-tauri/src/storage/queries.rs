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
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default = "default_granularity")]
    pub granularity: HistoryGranularity,
    pub iface: Option<String>,
}

impl HistoryQuery {
    pub(super) fn date_bounds(&self) -> Result<(Option<String>, Option<String>)> {
        let parse = |value: Option<&str>, name: &str| -> Result<Option<String>> {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    NaiveDate::parse_from_str(value, "%Y-%m-%d")
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .map_err(|error| AppError::Invalid(format!("{name} date: {error}")))
                })
                .transpose()
        };
        let from = parse(self.from.as_deref(), "from")?;
        let to = parse(self.to.as_deref(), "to")?;
        if matches!((&from, &to), (Some(from), Some(to)) if from > to) {
            return Err(AppError::Invalid("开始日期不能晚于结束日期".into()));
        }
        Ok((from, to))
    }
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
    let (from, to) = q.date_bounds()?;
    let iface = q
        .iface
        .as_deref()
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|error| AppError::Invalid(format!("iface luid: {error}")))
        })
        .transpose()?;
    let date_column = match q.granularity {
        HistoryGranularity::Day => "date_iso",
        HistoryGranularity::Month => "substr(date_iso,1,7)",
    };
    let mut filters = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(from) = from {
        filters.push("date_iso >= ?");
        params.push(from.into());
    }
    if let Some(to) = to {
        filters.push("date_iso <= ?");
        params.push(to.into());
    }
    if let Some(luid) = iface {
        filters.push("luid = ?");
        params.push(luid.into());
    }
    // Omit absent boundaries entirely: an empty range includes all stored dates,
    // while bounded queries retain simple predicates usable by the date index.
    let filter = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    let sql = format!(
        "SELECT {date_column}, SUM(bytes_recv), SUM(bytes_sent)
        FROM traffic_daily {filter} GROUP BY {date_column} ORDER BY 1 ASC"
    );
    let conn = pool.get().map_err(AppError::DbPool)?;
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok(DailyTraffic {
            date: row.get(0)?,
            iface: iface.map(|luid| luid.to_string()),
            bytes_recv: row.get(1)?,
            bytes_sent: row.get(2)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(AppError::from)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> DbPool {
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::memory())
            .unwrap();
        pool.get().unwrap().execute_batch(
            "CREATE TABLE traffic_daily(date_iso TEXT, luid INTEGER, bytes_recv INTEGER, bytes_sent INTEGER, PRIMARY KEY(date_iso,luid));
             INSERT INTO traffic_daily VALUES ('2001-01-01',1,10,1), ('2026-11-01',1,20,2),
             ('2026-11-02',2,30,3), ('2030-01-01',1,40,4);"
        ).unwrap();
        pool
    }

    #[test]
    fn empty_range_reads_all_existing_history_in_both_granularities() {
        let pool = pool();
        let mut query: HistoryQuery = serde_json::from_str(r#"{"from":null,"to":null}"#).unwrap();
        let daily = query_history(&pool, &query).unwrap();
        assert_eq!(daily.len(), 4);
        assert_eq!(daily.first().unwrap().date, "2001-01-01");
        assert_eq!(daily.last().unwrap().date, "2030-01-01");
        query.granularity = HistoryGranularity::Month;
        let monthly = query_history(&pool, &query).unwrap();
        assert_eq!(monthly.len(), 3);
        assert_eq!((monthly[1].bytes_recv, monthly[1].bytes_sent), (50, 5));
        query.iface = Some("1".into());
        assert_eq!(query_history(&pool, &query).unwrap()[1].bytes_recv, 20);
        query.granularity = HistoryGranularity::Day;
        assert_eq!(query_history(&pool, &query).unwrap().len(), 3);
        let omitted: HistoryQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query_history(&pool, &omitted).unwrap().len(), 4);
        let blank: HistoryQuery = serde_json::from_str(r#"{"from":" ","to":""}"#).unwrap();
        assert_eq!(query_history(&pool, &blank).unwrap().len(), 4);
    }

    #[test]
    fn optional_bounds_are_inclusive_and_invalid_dates_remain_errors() {
        let pool = pool();
        let mut query = HistoryQuery {
            from: Some("2026-11-01".into()),
            to: None,
            granularity: HistoryGranularity::Day,
            iface: None,
        };
        assert_eq!(query_history(&pool, &query).unwrap().len(), 3);
        query.from = None;
        query.to = Some("2026-11-01".into());
        assert_eq!(query_history(&pool, &query).unwrap().len(), 2);
        query.from = query.to.clone();
        assert_eq!(query_history(&pool, &query).unwrap().len(), 1);
        query.from = Some("2026-11-02".into());
        assert!(query_history(&pool, &query).is_err());
        query.from = Some("invalid".into());
        assert!(query_history(&pool, &query).is_err());
        query.from = None;
        query.to = Some("2026-02-30".into());
        assert!(query_history(&pool, &query).is_err());
    }
}
