pub mod v0_to_v1;

use crate::error::Result;
use rusqlite::Connection;

const CURRENT: u32 = 1;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;

    let from: u32 = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut v = from;
    while v < CURRENT {
        match v {
            0 => v0_to_v1::run(conn)?,
            _ => break,
        }
        v += 1;
    }

    conn.execute(
        "INSERT INTO meta(key,value) VALUES('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![CURRENT.to_string()],
    )?;
    Ok(())
}
