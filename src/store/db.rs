//! SQLite state database
//!
//! Manages the SQLite state database for tracking packages, versions, and file artifacts.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Result, params};
use thiserror::Error;

use crate::db_path;
use crate::store::history::HistoryEvent;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Package not found: {0}")]
    PackageNotFound(String),
}

/// Metadata for a specific package version stored in the database.
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub blake3: String,
    pub installed_at: i64,
    /// Whether this is the currently linked version
    pub active: bool,
    pub size_bytes: u64,
}

/// Artifact mapping (for a specific package version)
#[derive(Debug, Clone)]
pub struct Artifact {
    pub package: String,
    pub version: String,
    pub path: String, // Relative path (e.g. "bin/jq") usually, but absolute allowed for legacy migration
    pub blake3: String,
}

/// Installed file symlink (active on disk)
#[derive(Debug, Clone)]
pub struct InstalledFile {
    pub path: String, // Absolute path
    pub package: String,
    pub blake3: String,
}

pub struct StateDb {
    conn: Connection,
}

impl StateDb {
    /// Opens the default state database, initializing it if necessary.
    pub fn open() -> Result<Self, DbError> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        Self::open_at(&path)
    }

    /// Open database at a specific path (for testing)
    pub fn open_at(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;

        // Implementation Note: WAL and Foreign Keys
        //
        // 1. **WAL (Write-Ahead Logging)**: Greatly improves concurrency. Readers don't block writers,
        //    and writers don't block readers. This is crucial for a CLI that might run multiple
        //    instances (e.g., `apl install` in two terminals).
        //
        // 2. **Foreign Keys**: We enforce referential integrity at the DB level. If you delete a
        //    package, the DB ensures all its orphans (artifacts, files) are cleaned up or rejected.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let db = Self { conn };
        db.migrate_or_init()?;
        Ok(db)
    }

    fn migrate_or_init(&self) -> Result<(), DbError> {
        // 1. Check V2 (active column)
        let has_active: u32 = self
            .conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('packages') WHERE name='active'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if has_active == 0 {
            // Check if ANY table exists (if not, fresh init)
            let tables: u32 = self
                .conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='packages'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if tables > 0 {
                // V1 -> V2
                self.migrate_v1_to_v2()?;
            } else {
                // Fresh Init (includes V3)
                self.init_schema_v3()?;
                return Ok(());
            }
        }

        // 2. Check V3 (history)
        let has_history: u32 = self
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='history'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if has_history == 0 {
            self.migrate_v2_to_v3()?;
        }

        Ok(())
    }

    fn init_schema_v3(&self) -> Result<(), DbError> {
        // Includes V2 schema + V3 additions
        self.init_schema_v2()?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                action TEXT NOT NULL,
                package TEXT NOT NULL,
                version_from TEXT,
                version_to TEXT,
                success BOOLEAN NOT NULL DEFAULT 1
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_pkg ON history(package)",
            [],
        )?;
        Ok(())
    }

    fn migrate_v2_to_v3(&self) -> Result<(), DbError> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                action TEXT NOT NULL,
                package TEXT NOT NULL,
                version_from TEXT,
                version_to TEXT,
                success BOOLEAN NOT NULL DEFAULT 1
            )",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_pkg ON history(package)",
            [],
        )?;
        Ok(())
    }

    fn init_schema_v2(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS packages (
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                blake3 TEXT NOT NULL,
                installed_at INTEGER NOT NULL,
                active BOOLEAN NOT NULL DEFAULT 0,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (name, version)
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                package TEXT NOT NULL,
                version TEXT NOT NULL,
                path TEXT NOT NULL,
                blake3 TEXT NOT NULL,
                PRIMARY KEY (package, version, path),
                FOREIGN KEY(package, version) REFERENCES packages(name, version) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                package TEXT NOT NULL,
                blake3 TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_artifacts_pkg_ver ON artifacts(package, version);
            CREATE INDEX IF NOT EXISTS idx_files_package ON files(package);
            ",
        )?;
        Ok(())
    }

    fn migrate_v1_to_v2(&self) -> Result<(), DbError> {
        // Disable FKs during migration dance
        self.conn.execute_batch("PRAGMA foreign_keys=OFF;")?;

        // 1. Rename old tables
        self.conn.execute_batch(
            "
            ALTER TABLE packages RENAME TO packages_old;
            ALTER TABLE files RENAME TO files_old;
            ",
        )?;

        // 2. Create new schema
        self.init_schema_v2()?;

        // 3. Migrate Data
        // Assume all existing packages in V1 are 'active'
        self.conn.execute(
            "INSERT INTO packages (name, version, blake3, installed_at, active)
             SELECT name, version, blake3, installed_at, 1 FROM packages_old",
            [],
        )?;

        // Migrate files -> files (no FK now)
        self.conn.execute(
            "INSERT INTO files (path, package, blake3)
             SELECT path, package, blake3 FROM files_old",
            [],
        )?;

        // Backfill artifacts from files_old
        // Since V1 didn't track artifacts separately, we assume current files are the artifacts for current version
        self.conn.execute(
            "INSERT INTO artifacts (package, version, path, blake3)
             SELECT f.package, p.version, f.path, f.blake3 
             FROM files_old f 
             JOIN packages_old p ON f.package = p.name",
            [],
        )?;

        // 4. Cleanup
        self.conn.execute_batch(
            "
            DROP TABLE packages_old;
            DROP TABLE files_old;
            PRAGMA foreign_keys=ON;
            ",
        )?;
        Ok(())
    }

    /// Records a complete package installation atomically.
    ///
    /// This updates the package version record, artifact links, and current
    /// active files in a single transaction.
    pub fn install_complete_package(
        &self,
        name: &str,
        version: &str,
        blake3: &str,
        size_bytes: u64,
        artifacts: &[(String, String)],    // (path, blake3)
        active_files: &[(String, String)], // (path, blake3)
    ) -> Result<(), DbError> {
        // Implementation Note: Atomic Transactions
        //
        // We use a single transaction for all 4 distinct write operations.
        // If the power goes out after step 2, NO changes are persisted.
        // The database is always in a valid state: either the old package active, or the new one.
        // Never "half-installed".
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let tx = self.conn.unchecked_transaction()?;

        // 1. Deactivate others
        tx.execute(
            "UPDATE packages SET active = 0 WHERE name = ?1",
            params![name],
        )?;

        // 2. Insert package
        tx.execute(
            "INSERT OR REPLACE INTO packages (name, version, blake3, installed_at, active, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name, version, blake3, now, true, size_bytes],
        )?;

        // 3. Insert artifacts
        let mut stmt_art = tx.prepare("INSERT OR REPLACE INTO artifacts (package, version, path, blake3) VALUES (?1, ?2, ?3, ?4)")?;
        for (path, hash) in artifacts {
            stmt_art.execute(params![name, version, path, hash])?;
        }
        drop(stmt_art);

        // 4. Insert active files
        let mut stmt_file =
            tx.prepare("INSERT OR REPLACE INTO files (path, package, blake3) VALUES (?1, ?2, ?3)")?;
        for (path, hash) in active_files {
            stmt_file.execute(params![path, name, hash])?;
        }
        drop(stmt_file);

        tx.commit()?;
        Ok(())
    }

    /// Records a package version as active, deactivating any prior versions.
    pub fn install_package(&self, name: &str, version: &str, blake3: &str) -> Result<(), DbError> {
        self.install_package_version(name, version, blake3, true)
    }

    /// Inserts or updates a package version record.
    pub fn install_package_version(
        &self,
        name: &str,
        version: &str,
        blake3: &str,
        active: bool,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let tx = self.conn.unchecked_transaction()?; // using unchecked for simplicity with helper

        if active {
            // Deactivate other versions first
            tx.execute(
                "UPDATE packages SET active = 0 WHERE name = ?1",
                params![name],
            )?;
        }

        tx.execute(
            "INSERT OR REPLACE INTO packages (name, version, blake3, installed_at, active)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, version, blake3, now, active],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Record a file artifact for a package version
    pub fn add_artifact(
        &self,
        package: &str,
        version: &str,
        path: &str,
        blake3: &str,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO artifacts (package, version, path, blake3)
             VALUES (?1, ?2, ?3, ?4)",
            params![package, version, path, blake3],
        )?;
        Ok(())
    }

    /// Record an active installed file (symlink)
    pub fn add_file(&self, path: &str, package: &str, blake3: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, package, blake3)
             VALUES (?1, ?2, ?3)",
            params![path, package, blake3],
        )?;
        Ok(())
    }

    /// Removes all records for a package, returning the list of files to delete from disk.
    pub fn remove_package(&self, name: &str) -> Result<Vec<String>, DbError> {
        // Get active files to remove from disk
        let files = self.get_package_files(name)?;

        let tx = self.conn.unchecked_transaction()?;

        // Delete from all tables (manual cascade for files since FK removed)
        tx.execute("DELETE FROM files WHERE package = ?1", params![name])?;
        tx.execute("DELETE FROM artifacts WHERE package = ?1", params![name])?;
        let deleted = tx.execute("DELETE FROM packages WHERE name = ?1", params![name])?;

        tx.commit()?;

        if deleted == 0 {
            return Err(DbError::PackageNotFound(name.to_string()));
        }

        Ok(files.into_iter().map(|f| f.path).collect())
    }

    /// Retrieves the currently active version of a package.
    pub fn get_package(&self, name: &str) -> Result<Option<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at, active, size_bytes FROM packages WHERE name = ?1 AND active = 1",
        )?;

        let mut rows = stmt.query(params![name])?;

        if let Some(row) = rows.next()? {
            Ok(Some(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
                active: row.get(4)?,
                size_bytes: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get a specific version of a package
    pub fn get_package_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at, active, size_bytes FROM packages WHERE name = ?1 AND version = ?2",
        )?;

        let mut rows = stmt.query(params![name, version])?;

        if let Some(row) = rows.next()? {
            Ok(Some(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
                active: row.get(4)?,
                size_bytes: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all ACTIVE installed packages
    pub fn list_packages(&self) -> Result<Vec<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at, active, size_bytes FROM packages WHERE active = 1 ORDER BY name",
        )?;

        let packages = stmt.query_map([], |row| {
            Ok(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
                active: row.get(4)?,
                size_bytes: row.get(5)?,
            })
        })?;

        packages.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// List ALL installed versions of a package
    pub fn list_package_versions(&self, name: &str) -> Result<Vec<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at, active, size_bytes FROM packages WHERE name = ?1 ORDER BY version DESC",
        )?;

        let packages = stmt.query_map(params![name], |row| {
            Ok(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
                active: row.get(4)?,
                size_bytes: row.get(5)?,
            })
        })?;

        packages.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get all artifacts for a specific package version
    pub fn get_artifacts(&self, package: &str, version: &str) -> Result<Vec<Artifact>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT package, version, path, blake3 FROM artifacts WHERE package = ?1 AND version = ?2",
        )?;
        let rows = stmt.query_map(params![package, version], |row| {
            Ok(Artifact {
                package: row.get(0)?,
                version: row.get(1)?,
                path: row.get(2)?,
                blake3: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // History methods

    pub fn add_history(
        &self,
        package: &str,
        action: &str,
        version_from: Option<&str>,
        version_to: Option<&str>,
        success: bool,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        self.conn.execute(
            "INSERT INTO history (timestamp, action, package, version_from, version_to, success)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now, action, package, version_from, version_to, success],
        )?;
        Ok(())
    }

    pub fn get_history(&self, package: &str) -> Result<Vec<HistoryEvent>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, action, package, version_from, version_to, success 
             FROM history WHERE package = ?1 ORDER BY timestamp ASC",
        )?;
        let rows = stmt.query_map(params![package], |row| {
            Ok(HistoryEvent {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                action: row.get(2)?,
                package: row.get(3)?,
                version_from: row.get(4)?,
                version_to: row.get(5)?,
                success: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_last_successful_history(
        &self,
        package: &str,
    ) -> Result<Option<HistoryEvent>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, action, package, version_from, version_to, success 
             FROM history WHERE package = ?1 AND success = 1 ORDER BY timestamp DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![package], |row| {
            Ok(HistoryEvent {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                action: row.get(2)?,
                package: row.get(3)?,
                version_from: row.get(4)?,
                version_to: row.get(5)?,
                success: row.get(6)?,
            })
        })?;

        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    /// Get all active files for a package
    pub fn get_package_files(&self, package: &str) -> Result<Vec<InstalledFile>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, package, blake3 FROM files WHERE package = ?1")?;

        let files = stmt.query_map(params![package], |row| {
            Ok(InstalledFile {
                path: row.get(0)?,
                package: row.get(1)?,
                blake3: row.get(2)?,
            })
        })?;

        files.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Find which package owns a file
    pub fn find_file_owner(&self, path: &str) -> Result<Option<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT package FROM files WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;

        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_install_and_list() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        db.install_package("neovim", "0.10.0", "abc123").unwrap();
        db.install_package("ripgrep", "14.0.0", "def456").unwrap();

        let packages = db.list_packages().unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "neovim");
        assert!(packages[0].active);
        assert_eq!(packages[1].name, "ripgrep");
    }

    #[test]
    fn test_versions() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        // Install v1 (active)
        db.install_package("jq", "1.6", "abc").unwrap();
        let pkg = db.get_package("jq").unwrap().unwrap();
        assert_eq!(pkg.version, "1.6");
        assert!(pkg.active);

        // Install v2 (becomes active)
        db.install_package("jq", "1.7", "def").unwrap();
        let pkg = db.get_package("jq").unwrap().unwrap();
        assert_eq!(pkg.version, "1.7");
        assert!(pkg.active);

        // Check both exist
        let versions = db.list_package_versions("jq").unwrap();
        assert_eq!(versions.len(), 2);
        // Sorted desc
        assert_eq!(versions[0].version, "1.7");
        assert!(versions[0].active);
        assert_eq!(versions[1].version, "1.6");
        assert!(!versions[1].active);
    }

    #[test]
    fn test_file_tracking() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        db.install_package("neovim", "0.10.0", "abc123").unwrap();
        db.add_file("/usr/local/bin/nvim", "neovim", "file123")
            .unwrap();

        let owner = db.find_file_owner("/usr/local/bin/nvim").unwrap();
        assert_eq!(owner, Some("neovim".to_string()));
    }

    #[test]
    fn test_remove_package() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        db.install_package("neovim", "0.10.0", "abc123").unwrap();
        db.add_file("/usr/local/bin/nvim", "neovim", "file123")
            .unwrap();

        let files = db.remove_package("neovim").unwrap();
        assert_eq!(files, vec!["/usr/local/bin/nvim"]);

        assert!(db.get_package("neovim").unwrap().is_none());
    }
}
