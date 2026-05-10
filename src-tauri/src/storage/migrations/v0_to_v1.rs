use crate::error::Result;
use rusqlite::Connection;

pub fn run(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS interfaces (
            luid         INTEGER PRIMARY KEY,
            name         TEXT NOT NULL,
            description  TEXT NOT NULL,
            is_physical  INTEGER NOT NULL CHECK (is_physical IN (0,1)),
            first_seen   INTEGER NOT NULL,
            last_seen    INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS traffic_daily (
            date_iso   TEXT NOT NULL,
            luid       INTEGER NOT NULL,
            bytes_recv INTEGER NOT NULL DEFAULT 0,
            bytes_sent INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date_iso, luid)
        );

        CREATE INDEX IF NOT EXISTS idx_traffic_daily_date ON traffic_daily(date_iso);
        CREATE INDEX IF NOT EXISTS idx_traffic_daily_luid ON traffic_daily(luid);
        "#,
    )?;
    Ok(())
}
