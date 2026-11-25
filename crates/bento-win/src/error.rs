//! Error type of the interop layer.

/// Errors returned by `bento-win`.
#[derive(Debug, thiserror::Error)]
pub enum WinError {
    /// A Win32 or COM call failed.
    #[error("{context}: {source}")]
    Api {
        context: &'static str,
        source: windows::core::Error,
    },
    /// An expected window or object was not found.
    #[error("not found: {0}")]
    NotFound(&'static str),
    /// Any other failure with an explanation.
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, WinError>;

/// Adds a context to a `windows::core::Result`.
pub(crate) trait Context<T> {
    fn ctx(self, context: &'static str) -> Result<T>;
}

impl<T> Context<T> for windows::core::Result<T> {
    fn ctx(self, context: &'static str) -> Result<T> {
        self.map_err(|source| WinError::Api { context, source })
    }
}

/// Error for the last Win32 error code.
pub(crate) fn last_error(context: &'static str) -> WinError {
    WinError::Api {
        context,
        source: windows::core::Error::from_thread(),
    }
}
