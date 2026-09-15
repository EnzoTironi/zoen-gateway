//! SQLite catalog: WAL, pooled connections, indexed tool addresses.
//!
//! Disk IO is synchronous. Callers on a tokio runtime must use `spawn_blocking`.

#![allow(clippy::module_name_repetitions)] // `SqliteCatalog` is the crate's one type.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use executor_core::{
    CatalogStore, Connection, ConnectionRef, ExecutionId, ExecutionState, IntegrationRecord,
    IntegrationSlug, Owner, PolicyId, StorageError, Tool, ToolAddress, ToolPolicy,
};
use parking_lot::Mutex;
use rusqlite::{Connection as SqlConn, OptionalExtension, params};

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS integrations (
  slug TEXT PRIMARY KEY,
  body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connections (
  owner TEXT NOT NULL,
  integration TEXT NOT NULL,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  PRIMARY KEY (owner, integration, name)
);
CREATE INDEX IF NOT EXISTS idx_connections_integration ON connections(integration);
CREATE TABLE IF NOT EXISTS tools (
  address TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  integration TEXT NOT NULL,
  connection_name TEXT NOT NULL,
  static_tool INTEGER NOT NULL,
  body TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tools_conn ON tools(owner, integration, connection_name);
CREATE TABLE IF NOT EXISTS policies (
  id TEXT PRIMARY KEY,
  body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS executions (
  id TEXT PRIMARY KEY,
  body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS idempotency (
  key TEXT PRIMARY KEY,
  execution_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS kv (
  collection TEXT NOT NULL,
  id TEXT NOT NULL,
  body TEXT NOT NULL,
  PRIMARY KEY (collection, id)
);
";

/// Pooled SQLite-backed [`CatalogStore`].
pub struct SqliteCatalog {
    pool: Vec<Mutex<SqlConn>>,
    cursor: AtomicUsize,
}

impl SqliteCatalog {
    /// Open (or create) a database at `path` with `pool_size` connections.
    ///
    /// # Errors
    ///
    /// IO, sqlite, or empty pool.
    pub fn open(path: &Path, pool_size: usize) -> Result<Self, StorageError> {
        let size = pool_size.clamp(1, 32);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::new)?;
        }
        let mut pool = Vec::with_capacity(size);
        for _ in 0..size {
            let conn = SqlConn::open(path).map_err(StorageError::new)?;
            conn.execute_batch(SCHEMA).map_err(StorageError::new)?;
            pool.push(Mutex::new(conn));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)
                .map_err(StorageError::new)?
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(path, perms).map_err(StorageError::new)?;
        }
        Ok(Self {
            pool,
            cursor: AtomicUsize::new(0),
        })
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&SqlConn) -> Result<T, rusqlite::Error>,
    ) -> Result<T, StorageError> {
        let i = self.cursor.fetch_add(1, Ordering::Relaxed) % self.pool.len();
        let conn = self.pool[i].lock();
        f(&conn).map_err(StorageError::new)
    }
}

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

impl CatalogStore for SqliteCatalog {
    fn put_integration(&self, row: IntegrationRecord) -> Result<(), StorageError> {
        let body = serde_json::to_string(&row).map_err(|e| StorageError::new(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO integrations(slug, body) VALUES (?1, ?2)
                 ON CONFLICT(slug) DO UPDATE SET body=excluded.body",
                params![row.integration.slug.as_str(), body],
            )?;
            Ok(())
        })
    }

    fn get_integration(
        &self,
        slug: &IntegrationSlug,
    ) -> Result<Option<IntegrationRecord>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM integrations WHERE slug=?1",
                    params![slug.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn list_integrations(&self) -> Result<Vec<IntegrationRecord>, StorageError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT body FROM integrations")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?).map_err(json_err)?);
            }
            Ok(out)
        })
    }

    fn remove_integration(&self, slug: &IntegrationSlug) -> Result<bool, StorageError> {
        self.with(|c| {
            let n = c.execute(
                "DELETE FROM integrations WHERE slug=?1",
                params![slug.as_str()],
            )?;
            c.execute(
                "DELETE FROM connections WHERE integration=?1",
                params![slug.as_str()],
            )?;
            c.execute(
                "DELETE FROM tools WHERE integration=?1",
                params![slug.as_str()],
            )?;
            Ok(n > 0)
        })
    }

    fn put_connection(&self, row: Connection) -> Result<(), StorageError> {
        let body = serde_json::to_string(&row).map_err(|e| StorageError::new(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO connections(owner, integration, name, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(owner, integration, name) DO UPDATE SET body=excluded.body",
                params![
                    row.owner.as_str(),
                    row.integration.as_str(),
                    row.name.as_str(),
                    body
                ],
            )?;
            Ok(())
        })
    }

    fn get_connection(&self, id: &ConnectionRef) -> Result<Option<Connection>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM connections WHERE owner=?1 AND integration=?2 AND name=?3",
                    params![id.owner.as_str(), id.integration.as_str(), id.name.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn list_connections(
        &self,
        integration: Option<&IntegrationSlug>,
        owner: Option<Owner>,
    ) -> Result<Vec<Connection>, StorageError> {
        self.with(|c| {
            let mut sql = String::from("SELECT body FROM connections WHERE 1=1");
            if integration.is_some() {
                sql.push_str(" AND integration=?1");
            }
            if owner.is_some() {
                sql.push_str(if integration.is_some() {
                    " AND owner=?2"
                } else {
                    " AND owner=?1"
                });
            }
            let mut stmt = c.prepare(&sql)?;
            let mut bind: Vec<&str> = Vec::new();
            if let Some(i) = integration {
                bind.push(i.as_str());
            }
            if let Some(o) = owner {
                bind.push(o.as_str());
            }
            let rows =
                stmt.query_map(rusqlite::params_from_iter(bind), |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                let body = row?;
                out.push(serde_json::from_str(&body).map_err(json_err)?);
            }
            Ok(out)
        })
    }

    fn remove_connection(&self, id: &ConnectionRef) -> Result<bool, StorageError> {
        self.with(|c| {
            let n = c.execute(
                "DELETE FROM connections WHERE owner=?1 AND integration=?2 AND name=?3",
                params![id.owner.as_str(), id.integration.as_str(), id.name.as_str()],
            )?;
            c.execute(
                "DELETE FROM tools WHERE owner=?1 AND integration=?2 AND connection_name=?3 AND static_tool=0",
                params![id.owner.as_str(), id.integration.as_str(), id.name.as_str()],
            )?;
            Ok(n > 0)
        })
    }

    fn replace_tools(&self, id: &ConnectionRef, tools: Vec<Tool>) -> Result<(), StorageError> {
        self.with(|c| {
            c.execute(
                "DELETE FROM tools WHERE owner=?1 AND integration=?2 AND connection_name=?3 AND static_tool=0",
                params![id.owner.as_str(), id.integration.as_str(), id.name.as_str()],
            )?;
            let mut stmt = c.prepare(
                "INSERT INTO tools(address, owner, integration, connection_name, static_tool, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for tool in tools {
                let body = serde_json::to_string(&tool).map_err(json_err)?;
                stmt.execute(params![
                    tool.address.to_string(),
                    tool.owner.as_str(),
                    tool.integration.as_str(),
                    tool.connection.as_str(),
                    i32::from(tool.static_tool),
                    body
                ])?;
            }
            Ok(())
        })
    }

    fn get_tool(&self, address: &ToolAddress) -> Result<Option<Tool>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM tools WHERE address=?1",
                    params![address.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn list_tools(&self) -> Result<Vec<Tool>, StorageError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT body FROM tools")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?).map_err(json_err)?);
            }
            Ok(out)
        })
    }

    fn put_policy(&self, row: ToolPolicy) -> Result<(), StorageError> {
        let body = serde_json::to_string(&row).map_err(|e| StorageError::new(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO policies(id, body) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET body=excluded.body",
                params![row.id.as_str(), body],
            )?;
            Ok(())
        })
    }

    fn list_policies(&self) -> Result<Vec<ToolPolicy>, StorageError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT body FROM policies")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?).map_err(json_err)?);
            }
            Ok(out)
        })
    }

    fn remove_policy(&self, id: &PolicyId) -> Result<bool, StorageError> {
        self.with(|c| {
            let n = c.execute("DELETE FROM policies WHERE id=?1", params![id.as_str()])?;
            Ok(n > 0)
        })
    }

    fn put_execution(&self, id: &ExecutionId, state: ExecutionState) -> Result<(), StorageError> {
        let body = serde_json::to_string(&state).map_err(|e| StorageError::new(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO executions(id, body) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET body=excluded.body",
                params![id.as_str(), body],
            )?;
            Ok(())
        })
    }

    fn get_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM executions WHERE id=?1",
                    params![id.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn take_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM executions WHERE id=?1",
                    params![id.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            c.execute("DELETE FROM executions WHERE id=?1", params![id.as_str()])?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn put_idempotency(&self, key: &str, id: &ExecutionId) -> Result<(), StorageError> {
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO idempotency(key, execution_id) VALUES (?1, ?2)",
                params![key, id.as_str()],
            )?;
            Ok(())
        })
    }

    fn get_idempotency(&self, key: &str) -> Result<Option<ExecutionId>, StorageError> {
        self.with(|c| {
            let id: Option<String> = c
                .query_row(
                    "SELECT execution_id FROM idempotency WHERE key=?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            match id {
                None => Ok(None),
                Some(s) => {
                    Ok(Some(ExecutionId::new(&s).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                    })?))
                }
            }
        })
    }

    fn put_kv(
        &self,
        collection: &str,
        id: &str,
        body: serde_json::Value,
    ) -> Result<(), StorageError> {
        let text = serde_json::to_string(&body).map_err(|e| StorageError::new(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO kv(collection, id, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT(collection, id) DO UPDATE SET body=excluded.body",
                params![collection, id, text],
            )?;
            Ok(())
        })
    }

    fn get_kv(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        self.with(|c| {
            let body: Option<String> = c
                .query_row(
                    "SELECT body FROM kv WHERE collection=?1 AND id=?2",
                    params![collection, id],
                    |r| r.get(0),
                )
                .optional()?;
            match body {
                None => Ok(None),
                Some(b) => Ok(Some(serde_json::from_str(&b).map_err(json_err)?)),
            }
        })
    }

    fn list_kv(&self, collection: &str) -> Result<Vec<serde_json::Value>, StorageError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT body FROM kv WHERE collection=?1")?;
            let rows = stmt.query_map(params![collection], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?).map_err(json_err)?);
            }
            Ok(out)
        })
    }

    fn delete_kv(&self, collection: &str, id: &str) -> Result<bool, StorageError> {
        self.with(|c| {
            let n = c.execute(
                "DELETE FROM kv WHERE collection=?1 AND id=?2",
                params![collection, id],
            )?;
            Ok(n > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteCatalog;
    use executor_core::{CatalogStore, Integration, IntegrationRecord, IntegrationSlug, PluginId};

    #[test]
    fn roundtrip_integration() {
        let dir = tempfile::tempdir().unwrap();
        let db = SqliteCatalog::open(&dir.path().join("c.db"), 2).unwrap();
        let slug = IntegrationSlug::new("petstore").unwrap();
        let row = IntegrationRecord {
            integration: Integration {
                slug: slug.clone(),
                name: "Petstore".into(),
                description: "demo".into(),
                kind: PluginId::openapi(),
                can_remove: true,
                can_refresh: true,
                auth_methods: Vec::new(),
                display_url: None,
            },
            config: serde_json::json!({"baseUrl": "https://example.test"}),
        };
        db.put_integration(row).unwrap();
        let got = db.get_integration(&slug).unwrap().unwrap();
        assert_eq!(got.integration.name, "Petstore");
        assert!(db.remove_integration(&slug).unwrap());
        assert!(db.get_integration(&slug).unwrap().is_none());
    }
}
