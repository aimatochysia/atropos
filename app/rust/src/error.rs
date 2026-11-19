use thiserror::Error;

/// Every failure the core can report to the UI. The Dart side receives this as a
/// typed exception, so it can show a specific message for each case.
///
/// Crypto failures are deliberately vague: they never say *why* a key or
/// ciphertext was rejected, so error text cannot leak secret-dependent detail.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("network error: {message}")]
    Network { message: String },
    #[error("GitHub returned {status}: {message}")]
    GitHub { status: u16, message: String },
    #[error("update check failed: {message}")]
    Integrity { message: String },
    #[error("{message}")]
    InvalidInput { message: String },
    #[error("file error: {message}")]
    Io { message: String },
    #[error("decryption failed: wrong key or the data was changed")]
    DecryptFailed,
    #[error("cryptographic operation failed")]
    Crypto,
}

impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        CoreError::Io {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(err: serde_json::Error) -> Self {
        CoreError::Io {
            message: format!("settings file is invalid: {err}"),
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
