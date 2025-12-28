//! SQLite state database
//!
//! Tracks installed packages and their files.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Result, params};
use thiserror::Error;

use crate::db_path;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Package not found: {0}")]
    PackageNotFound(String),
}

/// Installed package record
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub blake3: String,
    pub installed_at: i64,
}

/// Installed file record
#[derive(Debug, Clone)]
pub struct InstalledFile {
    pub path: String,
    pub package: String,
    pub blake3: String,
}

/// State database for tracking installations
pub struct StateDb {
    conn: Connection,
}

impl StateDb {
    /// Open or create the state database
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

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS packages (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                blake3 TEXT NOT NULL,
                installed_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                package TEXT NOT NULL REFERENCES packages(name) ON DELETE CASCADE,
                blake3 TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_files_package ON files(package);
            ",
        )?;
        Ok(())
    }

    /// Record a package installation
    pub fn install_package(
        &self,
        name: &str,
        version: &str,
        blake3: &str,
    ) -> Result<(), DbError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT OR REPLACE INTO packages (name, version, blake3, installed_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![name, version, blake3, now],
        )?;
        Ok(())
    }

    /// Record an installed file
    pub fn add_file(&self, path: &str, package: &str, blake3: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files (path, package, blake3)
             VALUES (?1, ?2, ?3)",
            params![path, package, blake3],
        )?;
        Ok(())
    }

    /// Remove a package and its files
    pub fn remove_package(&self, name: &str) -> Result<Vec<String>, DbError> {
        // Get files to remove
        let files = self.get_package_files(name)?;

        // Delete files first (foreign key cascade would also work)
        self.conn.execute("DELETE FROM files WHERE package = ?1", params![name])?;

        // Delete package
        let deleted = self.conn.execute("DELETE FROM packages WHERE name = ?1", params![name])?;

        if deleted == 0 {
            return Err(DbError::PackageNotFound(name.to_string()));
        }

        Ok(files.into_iter().map(|f| f.path).collect())
    }

    /// Get a package by name
    pub fn get_package(&self, name: &str) -> Result<Option<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at FROM packages WHERE name = ?1",
        )?;

        let mut rows = stmt.query(params![name])?;

        if let Some(row) = rows.next()? {
            Ok(Some(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all installed packages
    pub fn list_packages(&self) -> Result<Vec<Package>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, blake3, installed_at FROM packages ORDER BY name",
        )?;

        let packages = stmt.query_map([], |row| {
            Ok(Package {
                name: row.get(0)?,
                version: row.get(1)?,
                blake3: row.get(2)?,
                installed_at: row.get(3)?,
            })
        })?;

        packages.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get all files for a package
    pub fn get_package_files(&self, package: &str) -> Result<Vec<InstalledFile>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, package, blake3 FROM files WHERE package = ?1",
        )?;

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
        let mut stmt = self.conn.prepare("SELECT package FROM files WHERE path = ?1")?;
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
        assert_eq!(packages[1].name, "ripgrep");
    }

    #[test]
    fn test_file_tracking() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        db.install_package("neovim", "0.10.0", "abc123").unwrap();
        db.add_file("/usr/local/bin/nvim", "neovim", "file123").unwrap();

        let owner = db.find_file_owner("/usr/local/bin/nvim").unwrap();
        assert_eq!(owner, Some("neovim".to_string()));
    }

    #[test]
    fn test_remove_package() {
        let dir = tempdir().unwrap();
        let db = StateDb::open_at(&dir.path().join("state.db")).unwrap();

        db.install_package("neovim", "0.10.0", "abc123").unwrap();
        db.add_file("/usr/local/bin/nvim", "neovim", "file123").unwrap();

        let files = db.remove_package("neovim").unwrap();
        assert_eq!(files, vec!["/usr/local/bin/nvim"]);

        assert!(db.get_package("neovim").unwrap().is_none());
    }
}
