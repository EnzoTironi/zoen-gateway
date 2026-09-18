//! Exclusive data-dir ownership (`BEGIN EXCLUSIVE` on a sidecar SQLite file).

use std::path::{Path, PathBuf};

use executor_core::StorageError;
use parking_lot::Mutex;
use rusqlite::{Connection, ErrorCode};

fn is_busy(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(code, _) => {
            matches!(
                code.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            )
        }
        other => {
            let msg = other.to_string();
            msg.contains("busy") || msg.contains("locked") || msg.contains("SQLITE_BUSY")
        }
    }
}

/// Held for the lifetime of a daemon process.
pub struct DataDirLock {
    conn: Mutex<Connection>,
    /// Absolute lock-file path.
    pub lock_path: PathBuf,
}

impl DataDirLock {
    /// Try to own `data_dir`. Fail-fast if another process already holds it.
    ///
    /// # Errors
    ///
    /// IO, sqlite, or [`StorageError`] when the lock is busy.
    pub fn acquire(data_dir: &Path) -> Result<Self, StorageError> {
        std::fs::create_dir_all(data_dir).map_err(StorageError::new)?;
        let lock_path = data_dir.join("data.db.owner-lock");
        let conn = Connection::open(&lock_path).map_err(StorageError::new)?;
        conn.execute_batch("PRAGMA busy_timeout = 0; PRAGMA journal_mode = DELETE;")
            .map_err(StorageError::new)?;
        match conn.execute_batch("BEGIN EXCLUSIVE") {
            Ok(()) => Ok(Self {
                conn: Mutex::new(conn),
                lock_path,
            }),
            Err(err) if is_busy(&err) => Err(StorageError::new(format!(
                "Executor data directory is already owned by another process: {}",
                lock_path.display()
            ))),
            Err(err) => Err(StorageError::new(err)),
        }
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        let conn = self.conn.lock();
        let _ = conn.execute_batch("ROLLBACK");
    }
}

#[cfg(test)]
mod tests {
    use super::DataDirLock;

    #[test]
    fn second_owner_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let first = DataDirLock::acquire(dir.path()).unwrap();
        let second = DataDirLock::acquire(dir.path());
        assert!(second.is_err(), "expected busy lock");
        drop(first);
        DataDirLock::acquire(dir.path()).unwrap();
    }
}
