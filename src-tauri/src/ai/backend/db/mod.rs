//! AI 管理 SQLite 基础设施：r2d2 连接池、统一 PRAGMA 与 schema 迁移。

pub mod migrations;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub type DbPool = Pool<SqliteConnectionManager>;

/// 每个新连接统一设置的 PRAGMA。
fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(())
}

/// 默认数据库路径由 SysPulse AI 模块配置。
pub fn default_db_path() -> PathBuf {
    crate::ai::backend::runtime::data_dir().join("ai-tools.db")
}

/// 用指定路径建立连接池并完成迁移。
pub fn init_at(path: &Path) -> Result<DbPool, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建数据库目录失败: {e}"))?;
    }
    let manager = SqliteConnectionManager::file(path).with_init(|c| apply_pragmas(c));
    let pool = Pool::builder()
        .build(manager)
        .map_err(|e| format!("创建连接池失败: {e}"))?;

    let conn = pool.get().map_err(|e| format!("获取连接失败: {e}"))?;
    migrations::run(&conn).map_err(|e| format!("schema 迁移失败: {e}"))?;
    Ok(pool)
}

/// 用默认路径初始化。
pub fn init() -> Result<DbPool, String> {
    init_at(&default_db_path())
}

static POOL: OnceLock<DbPool> = OnceLock::new();

/// 启动期显式初始化全局连接池（可提前暴露错误）；幂等。
/// 即使未调用，`pool()` 仍会按默认路径懒初始化。
pub fn init_global() -> Result<(), String> {
    if POOL.get().is_some() {
        return Ok(());
    }
    let p = init()?;
    let _ = POOL.set(p);
    Ok(())
}

/// 获取全局连接池：若未显式初始化则按默认路径懒初始化（生产/测试通用）。
pub fn pool() -> Result<&'static DbPool, String> {
    if POOL.get().is_none() {
        let p = init()?;
        let _ = POOL.set(p);
    }

    POOL.get()
        .ok_or_else(|| "数据库连接池初始化失败".to_string())
}

pub fn connection() -> Result<PooledConnection<SqliteConnectionManager>, String> {
    pool()?
        .get()
        .map_err(|e| format!("获取数据库连接失败: {e}"))
}

/// 生成数据库的一致性单文件快照（`VACUUM INTO`）。
/// WAL 模式下安全：快照包含全部已提交数据，无需另拷 `-wal`/`-shm` 伴生文件。
/// 目标文件必须不存在。
pub fn backup_to(dest: &Path) -> Result<(), String> {
    if dest.exists() {
        return Err(format!("备份目标已存在: {}", dest.display()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建备份目录失败: {e}"))?;
    }
    let conn = connection()?;
    // VACUUM INTO 不支持参数绑定，目标路径以字符串字面量拼接（转义单引号防注入）
    let dest_lit = dest.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{dest_lit}'"))
        .map_err(|e| format!("数据库备份失败: {e}"))
}

/// 读取 KV 表中某 key 的值（table 为内部常量，无注入风险）。
pub fn kv_get(table: &str, key: &str) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension;
    let conn = connection()?;
    conn.query_row(
        &format!("SELECT value FROM {table} WHERE key=?1"),
        [key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("读取 {table} 失败: {e}"))
}

/// 写入 KV 表（按 key upsert）。
pub fn kv_set(table: &str, key: &str, value: &str) -> Result<(), String> {
    let conn = connection()?;
    conn.execute(
        &format!(
            "INSERT INTO {table}(key,value) VALUES(?1,?2) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value"
        ),
        rusqlite::params![key, value],
    )
    .map(|_| ())
    .map_err(|e| format!("写入 {table} 失败: {e}"))
}

/// 敏感数据从明文迁移后重建数据库并截断 WAL，避免旧页继续残留明文。
pub fn compact_after_sensitive_migration() -> Result<(), String> {
    let conn = connection()?;
    conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| format!("清理敏感数据迁移残留失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_init_runs_ai_migrations() {
        let dir = std::env::temp_dir().join(format!("syspulse_ai_test_{}", std::process::id()));
        let path = dir.join("t.db");
        let _ = std::fs::remove_dir_all(&dir);
        let pool = init_at(&path).unwrap();
        let conn = pool.get().unwrap();

        // 迁移已应用
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, migrations::MIGRATIONS_LEN as i64);

        conn.execute("INSERT INTO mcp_oauth(key,value) VALUES('store','{}')", [])
            .unwrap();
        let value: String = conn
            .query_row("SELECT value FROM mcp_oauth WHERE key='store'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "{}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
