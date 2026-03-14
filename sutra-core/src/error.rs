//! Error types for sutra-core.

use thiserror::Error;

/// Errors that can occur in the core triple storage engine.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An IRI string was invalid.
    #[error("invalid IRI: {0}")]
    InvalidIri(String),

    /// A triple referenced an ID that does not exist in the dictionary.
    #[error("unknown ID: {0}")]
    UnknownId(u64),

    /// Attempted to insert a duplicate triple.
    #[error("duplicate triple")]
    DuplicateTriple,

    /// Storage I/O error.
    #[error("storage error: {0}")]
    Storage(#[from] std::io::Error),

    /// Sled storage error.
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
