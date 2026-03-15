use thiserror::Error;

/// Errors that can occur when communicating with a SutraDB instance.
#[derive(Debug, Error)]
pub enum SutraError {
    /// An HTTP-level error occurred (connection refused, timeout, etc.).
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// The server returned a non-success status code or an error payload.
    #[error("SutraDB server error ({status}): {message}")]
    Server {
        status: u16,
        message: String,
    },

    /// Failed to deserialize the server response.
    #[error("deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),
}

/// A convenience alias for `std::result::Result<T, SutraError>`.
pub type Result<T> = std::result::Result<T, SutraError>;
