//! Core modules - pure, stateless logic

pub mod package;
pub mod index;
pub mod lockfile;
pub mod resolver;
pub mod version;

// Backwards compatibility alias
pub use package as formula;
