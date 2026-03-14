//! Error types for sutra-sparql.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SparqlError {
    #[error("parse error at position {position}: {message}")]
    Parse { position: usize, message: String },

    #[error("unknown prefix: {0}")]
    UnknownPrefix(String),

    #[error("query execution error: {0}")]
    Execution(String),

    #[error("core error: {0}")]
    Core(#[from] sutra_core::CoreError),
}

pub type Result<T> = std::result::Result<T, SparqlError>;
