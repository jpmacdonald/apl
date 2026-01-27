//! Shared installation context.
//!
//! This module defines the `Context` struct, which groups common state references
//! used throughout the installation process to reduce argument fatigue.

use crate::DbHandle;
use crate::ui::Reporter;
use apl_schema::index::PackageIndex;
use std::fmt;
use std::sync::Arc;

/// Groups common state used during installation operations.
#[derive(Clone)]
pub struct Context {
    pub db: DbHandle,
    pub index: Option<Arc<PackageIndex>>,
    pub client: reqwest::Client,
    pub reporter: Arc<dyn Reporter>,
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl Context {
    pub fn new(
        db: DbHandle,
        index: Option<PackageIndex>,
        client: reqwest::Client,
        reporter: Arc<dyn Reporter>,
    ) -> Self {
        Self {
            db,
            index: index.map(Arc::new),
            client,
            reporter,
        }
    }
}
