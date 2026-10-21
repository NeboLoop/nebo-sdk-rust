/// SDK error type for Nebo app operations.
#[derive(Debug, thiserror::Error)]
pub enum NeboError {
    #[error("NEBO_APP_TOKEN environment variable is not set (Nebo sets it when it launches the app)")]
    NoToken,

    #[error("no capability handlers registered")]
    NoHandlers,

    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("execution error: {0}")]
    Execution(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl From<String> for NeboError {
    fn from(s: String) -> Self {
        NeboError::Other(s)
    }
}

impl From<&str> for NeboError {
    fn from(s: &str) -> Self {
        NeboError::Other(s.to_string())
    }
}
