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
    #[error(
        "no credential profile is selected; pass --profile, set LURKLINE_PROFILE, or import a profile"
    )]
    MissingProfile,
    #[error("credential profile {profile} was not found")]
    ProfileNotFound { profile: String },
    #[error("credential profile registry is invalid; repair or remove it before continuing")]
    InvalidProfileRegistry,
    #[error("credential profile registry could not be read")]
    ProfileRegistryRead,
    #[error("credential profile registry could not be written")]
    ProfileRegistryWrite,
    #[error(
        "the operating system credential store is unavailable; configure all four SLACK_* environment variables instead"
    )]
    CredentialStoreUnavailable,
    #[error(
        "the operating system credential store operation failed; unlock or configure it, or use all four SLACK_* environment variables"
    )]
    CredentialStore,
    #[error("stored credentials for profile {profile} are invalid; re-import the profile")]
    InvalidStoredCredential { profile: String },
    #[error(
        "stored credentials for profile {profile} do not match its registry metadata; re-import the profile"
    )]
    CredentialProfileMismatch { profile: String },
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
    #[error("could not resolve {resource} within the {limit}-item scan limit")]
    ScanLimit {
        resource: &'static str,
        limit: usize,
    },
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
