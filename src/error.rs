use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing required environment variable {0}")]
    MissingConfig(&'static str),
    #[error("invalid {name}: {reason}")]
    InvalidConfig { name: &'static str, reason: String },
    #[error("invalid {field}: {reason}")]
    InvalidInput {
        field: &'static str,
        reason: &'static str,
    },
    #[error("Slack browser session is expired or invalid; copy fresh browser credentials")]
    Authentication,
    #[error("Slack method {method} failed: {code}")]
    SlackApi { method: &'static str, code: String },
    #[error("Slack method {method} returned HTTP {status}")]
    HttpStatus { method: &'static str, status: u16 },
    #[error("Slack method {method} returned a response larger than {limit} bytes")]
    ResponseTooLarge { method: &'static str, limit: usize },
    #[error("Slack method {method} returned an invalid response")]
    InvalidResponse { method: &'static str },
    #[error("Slack method {method} timed out")]
    Timeout { method: &'static str },
    #[error("Slack method {method} could not be reached")]
    Transport { method: &'static str },
    #[error("{resource} was not found")]
    NotFound { resource: &'static str },
    #[error("could not serialize output")]
    Output,
    #[error("MCP stdio transport failed")]
    McpTransport,
}

impl Error {
    pub(crate) fn invalid_config(name: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidConfig {
            name,
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_input(field: &'static str, reason: &'static str) -> Self {
        Self::InvalidInput { field, reason }
    }
}
