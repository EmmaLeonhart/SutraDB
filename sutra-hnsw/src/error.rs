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

    /// A vector index has already been declared for this predicate.
    #[error("predicate already declared as vector index: {0}")]
    PredicateAlreadyDeclared(u64),

    /// No vector index exists for this predicate.
    #[error("no vector index for predicate: {0}")]
    NoIndexForPredicate(u64),
}

pub type Result<T> = std::result::Result<T, HnswError>;
