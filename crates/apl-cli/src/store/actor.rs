//! DB Actor - Thread-safe access to SQLite
//!
//! This module implements the actor pattern for the state database.
//! Since SQLite connections are not thread-safe (not Sync), we host
//! the database handle in a dedicated background thread and communicate
//! via message passing.

use std::fmt;
use std::sync::mpsc;
use std::thread;
use tokio::sync::oneshot;

use super::db::{DbError, InstalledFile, Package, StateDb};

/// Events that can be sent to the DB actor
pub enum DbEvent {
    /// Get currently active version of a package
    GetPackage {
        name: String,
        resp: oneshot::Sender<Result<Option<Package>, DbError>>,
    },
    /// Get all files tracked for a package
    GetPackageFiles {
        name: String,
        resp: oneshot::Sender<Result<Vec<InstalledFile>, DbError>>,
    },
    /// Get a specific version of a package
    GetPackageVersion {
        name: String,
        version: String,
        resp: oneshot::Sender<Result<Option<Package>, DbError>>,
    },
    /// Remove a package and its file records
    RemovePackage {
        name: String,
        resp: oneshot::Sender<Result<Vec<String>, DbError>>,
    },
    /// Add an entry to the installation history
    AddHistory {
        name: String,
        action: String,
        version: Option<String>,
        sha256: Option<String>,
        success: bool,
        resp: oneshot::Sender<Result<(), DbError>>,
    },
    /// Record a complete package installation
    InstallComplete {
        name: String,
        version: String,
        sha256: String,
        size_bytes: u64,
        artifacts: Vec<(String, String)>,
        active_files: Vec<(String, String)>,
        resp: oneshot::Sender<Result<(), DbError>>,
    },
    /// Shutdown the actor
    Shutdown,
}

impl fmt::Debug for DbEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GetPackage { name, .. } => f
                .debug_struct("GetPackage")
                .field("name", name)
                .finish_non_exhaustive(),
            Self::GetPackageFiles { name, .. } => f
                .debug_struct("GetPackageFiles")
                .field("name", name)
                .finish_non_exhaustive(),
            Self::GetPackageVersion { name, version, .. } => f
                .debug_struct("GetPackageVersion")
                .field("name", name)
                .field("version", version)
                .finish_non_exhaustive(),
            Self::RemovePackage { name, .. } => f
                .debug_struct("RemovePackage")
                .field("name", name)
                .finish_non_exhaustive(),
            Self::AddHistory {
                name,
                action,
                version,
                ..
            } => f
                .debug_struct("AddHistory")
                .field("name", name)
                .field("action", action)
                .field("version", version)
                .finish_non_exhaustive(),
            Self::InstallComplete { name, version, .. } => f
                .debug_struct("InstallComplete")
                .field("name", name)
                .field("version", version)
                .finish_non_exhaustive(),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// A handle to the Database Actor that is Send + Sync and Clone.
#[derive(Clone)]
pub struct DbHandle {
    sender: mpsc::Sender<DbEvent>,
}

impl fmt::Debug for DbHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbHandle").finish_non_exhaustive()
    }
}

impl DbHandle {
    /// Spawn a new DB actor thread
    pub fn spawn() -> Result<Self, DbError> {
        let (sender, receiver) = mpsc::channel();
        let db = StateDb::open()?;

        thread::spawn(move || {
            run_db_event_loop(db, receiver);
        });

        Ok(Self { sender })
    }

    /// Helper to send a request and wait for the response
    async fn request<T, F>(&self, f: F) -> Result<T, DbError>
    where
        F: FnOnce(oneshot::Sender<Result<T, DbError>>) -> DbEvent,
    {
        let (tx, rx) = oneshot::channel();
        self.sender.send(f(tx)).map_err(|_| DbError::ActorDied)?;
        rx.await.map_err(|_| DbError::ActorDied)?
    }

    pub async fn get_package(&self, name: String) -> Result<Option<Package>, DbError> {
        self.request(|resp| DbEvent::GetPackage { name, resp })
            .await
    }

    pub async fn get_package_files(&self, name: String) -> Result<Vec<InstalledFile>, DbError> {
        self.request(|resp| DbEvent::GetPackageFiles { name, resp })
            .await
    }

    pub async fn get_package_version(
        &self,
        name: String,
        version: String,
    ) -> Result<Option<Package>, DbError> {
        self.request(|resp| DbEvent::GetPackageVersion {
            name,
            version,
            resp,
        })
        .await
    }

    pub async fn remove_package(&self, name: String) -> Result<Vec<String>, DbError> {
        self.request(|resp| DbEvent::RemovePackage { name, resp })
            .await
    }

    pub async fn add_history(
        &self,
        name: String,
        action: String,
        version: Option<String>,
        sha256: Option<String>,
        success: bool,
    ) -> Result<(), DbError> {
        self.request(|resp| DbEvent::AddHistory {
            name,
            action,
            version,
            sha256,
            success,
            resp,
        })
        .await
    }

    pub async fn install_complete_package(
        &self,
        name: String,
        version: String,
        sha256: String,
        size_bytes: u64,
        artifacts: Vec<(String, String)>,
        active_files: Vec<(String, String)>,
    ) -> Result<(), DbError> {
        self.request(|resp| DbEvent::InstallComplete {
            name,
            version,
            sha256,
            size_bytes,
            artifacts,
            active_files,
            resp,
        })
        .await
    }
}

/// The actual event loop running in the background thread
// The db and receiver are intentionally moved into this thread to ensure
// exclusive ownership for the actor pattern.
#[allow(clippy::needless_pass_by_value)]
fn run_db_event_loop(db: StateDb, receiver: mpsc::Receiver<DbEvent>) {
    while let Ok(event) = receiver.recv() {
        match event {
            DbEvent::GetPackage { name, resp } => {
                let _ = resp.send(db.get_package(&name));
            }
            DbEvent::GetPackageFiles { name, resp } => {
                let _ = resp.send(db.get_package_files(&name));
            }
            DbEvent::GetPackageVersion {
                name,
                version,
                resp,
            } => {
                let _ = resp.send(db.get_package_version(&name, &version));
            }
            DbEvent::RemovePackage { name, resp } => {
                let _ = resp.send(db.remove_package(&name));
            }
            DbEvent::AddHistory {
                name,
                action,
                version,
                sha256,
                success,
                resp,
            } => {
                let _ = resp.send(db.add_history(
                    &name,
                    &action,
                    version.as_deref(),
                    sha256.as_deref(),
                    success,
                ));
            }
            DbEvent::InstallComplete {
                name,
                version,
                sha256,
                size_bytes,
                artifacts,
                active_files,
                resp,
            } => {
                let _ = resp.send(db.install_complete_package(
                    &name,
                    &version,
                    &sha256,
                    size_bytes,
                    &artifacts,
                    &active_files,
                ));
            }
            DbEvent::Shutdown => break,
        }
    }
}
