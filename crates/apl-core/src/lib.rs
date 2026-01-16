pub mod builder;
pub mod indexer;
pub mod io;
pub mod manifest;
pub mod package;
pub mod paths;
pub mod pubgrub_adapter;
pub mod relinker;
pub mod repo;
pub mod resolver;
pub mod strategies;
pub mod sysroot;
pub mod types;

pub mod reporter;

// Re-export Strategy trait if needed, or let users import from strategies
pub use paths::*;
pub use reporter::{NullReporter, Reporter};
pub use strategies::Strategy;

/// User Agent string for core operations
pub const USER_AGENT: &str = concat!("apl-core/", env!("CARGO_PKG_VERSION"));
