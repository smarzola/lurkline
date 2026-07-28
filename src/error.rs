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
    #[error("credentials for profile {profile} are missing; re-import the profile")]
    MissingProfileCredential { profile: String },
    #[error(
        "credential profile {profile} belongs to another workspace; pass --replace-workspace to replace it"
    )]
    ProfileWorkspaceMismatch { profile: String },
    #[error("credential profile registry is invalid; repair or remove it before continuing")]
    InvalidProfileRegistry,
    #[error("credential profile registry could not be read")]
    ProfileRegistryRead,
    #[error("credential profile registry could not be written")]
    ProfileRegistryWrite,
    #[error("credential profile registry could not be locked")]
    ProfileRegistryLock,
    #[error(
        "credential storage operation failed; re-import the profile or use all four SLACK_* environment variables"
    )]
    CredentialStorage,
    #[error("unsafe {resource}; it must be owned by the current user with owner-only permissions")]
    UnsafeCredentialStorage { resource: &'static str },
    #[error(
        "stored credentials for profile {profile} exceed the size limit; re-import the profile"
    )]
    CredentialTooLarge { profile: String },
    #[error("stored credentials for profile {profile} are invalid; re-import the profile")]
    InvalidStoredCredential { profile: String },
    #[error(
        "stored credentials for profile {profile} do not match its registry metadata; re-import the profile"
    )]
    CredentialProfileMismatch { profile: String },
    #[error(
        "credential update for profile {profile} could not be rolled back; re-import the profile before use"
    )]
    CredentialReconciliation { profile: String },
    #[error("could not read cURL input from standard input")]
    InputRead,
    #[error("could not read Markdown input from standard input")]
    MarkdownInputRead,
    #[error("Slack writes are disabled; start the MCP server with --allow-write")]
    WriteNotAllowed,
    #[error("local file tools are disabled; start the MCP server with --file-root ABSOLUTE_PATH")]
    FileRootRequired,
    #[error("confirmation is required for {action}")]
    ConfirmationRequired { action: &'static str },
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
    #[error(
        "Slack publication outcome is unknown for client message {client_msg_id}; do not retry automatically; verify the message in Slack before deciding whether to retry"
    )]
    PublicationUncertain { client_msg_id: String },
    #[error(
        "Slack reaction outcome is unknown for {channel_id} at {message_ts} ({name}); do not retry automatically; read the exact message before deciding whether to retry"
    )]
    ReactionUncertain {
        channel_id: String,
        message_ts: String,
        name: String,
    },
    #[error(
        "Slack reaction is confirmed not applied for {channel_id} at {message_ts} ({name}); the exact state is known and a deliberate retry is safe"
    )]
    ReactionNotApplied {
        channel_id: String,
        message_ts: String,
        name: String,
    },
    #[error("local file operation failed: {operation}")]
    LocalFile { operation: String },
    #[error("{resource} was not found")]
    NotFound { resource: &'static str },
    #[error("could not resolve {resource} within the {limit}-item scan limit")]
    ScanLimit {
        resource: &'static str,
        limit: usize,
    },
    #[error("could not serialize output")]
    Output,
    #[error("system clock is earlier than the Unix epoch")]
    SystemClock,
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
