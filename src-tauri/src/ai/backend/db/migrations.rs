//! AI 管理专用 SQLite schema。

use rusqlite::Connection;

#[allow(dead_code)]
pub const MIGRATIONS_LEN: usize = MIGRATIONS.len();

const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE IF NOT EXISTS mcp_oauth (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#];

pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let target = MIGRATIONS.len() as i64;
    for version in current..target {
        conn.execute_batch(MIGRATIONS[version as usize])?;
    }
    if target > current {
        conn.execute_batch(&format!("PRAGMA user_version = {target};"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_create_only_required_oauth_store() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        let tables: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(tables, vec!["mcp_oauth"]);
    }

    #[test]
    fn oauth_store_supports_upsert() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        for value in ["v1", "v2"] {
            conn.execute(
                "INSERT INTO mcp_oauth(key,value) VALUES('store',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )
            .unwrap();
        }
        let value: String = conn
            .query_row("SELECT value FROM mcp_oauth WHERE key='store'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "v2");
    }
}
