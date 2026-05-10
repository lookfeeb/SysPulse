use crate::error::{AppError, Result};
use crate::storage::migrations;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;
use std::path::Path;
use std::sync::Arc;

pub type DbPool = Pool<SqliteConnectionManager>;

pub struct TrafficStore {
    pool: DbPool,
}

impl TrafficStore {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        let manager = SqliteConnectionManager::file(path)
            .with_flags(OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE);
        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .map_err(AppError::DbPool)?;
        {
            let conn = pool.get().map_err(AppError::DbPool)?;
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous  = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )?;
            migrations::migrate(&conn)?;
        }
        Ok(Arc::new(Self { pool }))
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
}
