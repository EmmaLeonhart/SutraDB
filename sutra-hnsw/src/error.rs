//! Error types for sutra-hnsw.

use thiserror::Error;

/// Errors that can occur in the HNSW index.
#[derive(Debug, Error)]
pub enum HnswError {
    /// Vector dimension does not match the index's declared dimension.
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    /// The given triple ID was not found in the index.
    #[error("triple ID not found in index: {0}")]
    NotFound(u64),

    /// The index is empty and cannot be searched.
    #[error("index is empty")]
    EmptyIndex,
}

pub type Result<T> = std::result::Result<T, HnswError>;
