use std::path::PathBuf;

/// Errors that can occur during symbol extraction.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("parse failed for {path}")]
    ParseFailed { path: PathBuf },

    #[error("query error in {query_name}: {message}")]
    QueryError {
        query_name: &'static str,
        message: String,
    },
}
