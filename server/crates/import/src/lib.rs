//! freeLib web: INPX import into `lib_<id>.db`, zip offset resolution, migration from the
//! Qt `freeLib.sqlite`, and a synthetic INPX generator used by tests and benchmarks.

pub mod builder;
pub mod synth;
pub mod inpx;
pub mod migrate;
pub mod offsets;
pub mod zipdir;

pub use builder::{import_inpx, new_db_path, ImportOptions, ImportStats, Progress};
pub use offsets::{resolve_offsets, OffsetStats};

/// Import errors.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("import cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}
