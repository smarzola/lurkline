use std::{env, fmt::Write as _, io::Read};

use clap::{Parser, Subcommand};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{
    auth::{
        AuthImportReport, AuthListReport, AuthRemoveReport, AuthStatusReport, ProfileName,
        list_profiles, profile_status, remove_profile, resolve_config, store_profile,
    },
    config::Config,
    curl_import::{MAX_CURL_BYTES, parse_copy_as_curl},
    error::{Error, Result},
    http::SlackHttpClient,
    local_file::{
        prepare_cli_download, prepare_cli_upload, validate_cli_download_path,
        validate_cli_upload_path,
    },
    markdown::{MAX_MARKDOWN_BYTES, render_markdown},
    mcp,
    model::{
        Conversation, ConversationKind, ConversationPage, ConversationSearchReport,
        ConversationSearchTruncationReason, CustomEmojiKind, CustomEmojiList, DoctorReport, Draft,
        DraftDeleteReport, DraftPage, DraftSendReport, FileDownloadReport, FileDraftAssociation,
        FileDraftCreateReport, FileReference, FileUploadReport, InboxReport, InboxTruncationReason,
        Message, MessagePage, MessageSearchPage, ReactionMutationReport, RenderedMessage,
        SentMessage, ThreadPage, UnreadReport, UserSearchReport, UserSearchTruncationReason,
    },
    service::{
        DEFAULT_FILE_DOWNLOAD_BYTES, DEFAULT_FILE_UPLOAD_BYTES, FileDraftCreateRequest,
        MAX_CONVERSATIONS, MAX_FILE_DOWNLOAD_BYTES, MAX_FILE_UPLOAD_BYTES, MAX_USERS, SlackService,
    },
};

#[derive(Debug, Parser)]
#[command(
    name = "lurkline",
    version,
    about = "Slack reading and guarded rich-text authoring through an existing browser session"
)]
pub struct Cli {
    /// Use a named credential profile. LURKLINE_PROFILE is the fallback.
    #[arg(long, global = true)]
    pub profile: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Import and manage browser-session credential profiles.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Validate configuration and probe the Slack browser session.
    Doctor {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List conversations and thread counts Slack marks unread.
    Unreads {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read a bounded snapshot of conversations Slack explicitly marks unread.
    Inbox {
        /// Maximum unread conversations to load, from 1 through 50.
        #[arg(long = "conversations", default_value_t = 10)]
        conversation_limit: usize,
        /// Maximum recent messages to load per unread conversation, from 1 through 200.
        #[arg(long = "messages", default_value_t = 20)]
        message_limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Discover channels, DMs, and group DMs by human-readable name.
    Conversations {
        #[command(subcommand)]
        command: ConversationsCommand,
    },
    /// Search workspace messages with bounded filters.
    Search {
        #[command(subcommand)]
        command: SearchCommand,
    },
    /// Read messages from a channel, DM, or group DM.
    Channel {
        #[command(subcommand)]
        command: ChannelCommand,
    },
    /// Read or reply to a message thread.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// Fetch, render, or send a message.
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },
    /// List and manage Slack drafts.
    Drafts {
        #[command(subcommand)]
        command: DraftsCommand,
    },
    /// Inspect and download private Slack files.
    Files {
        #[command(subcommand)]
        command: FilesCommand,
    },
    /// Discover workspace custom emoji.
    Emoji {
        #[command(subcommand)]
        command: EmojiCommand,
    },
    /// Add or remove emoji reactions on exact messages.
    Reactions {
        #[command(subcommand)]
        command: ReactionsCommand,
    },
    /// Find workspace users.
    Users {
        #[command(subcommand)]
        command: UsersCommand,
    },
    /// Run the MCP server over stdin/stdout.
    Mcp {
        /// Enable draft and message write tools. Reads remain available by default.
        #[arg(long)]
        allow_write: bool,
        /// Absolute local directory exposed to file tools.
        #[arg(long)]
        file_root: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum FilesCommand {
    /// Fetch bounded metadata for one Slack file.
    Info {
        /// Slack file identifier.
        file_id: String,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Download one private Slack file without overwriting an existing path.
    Download {
        /// Slack file identifier.
        file_id: String,
        /// Explicit local output path.
        #[arg(long)]
        output: std::path::PathBuf,
        /// Maximum bytes to write, up to 1 GiB.
        #[arg(long, default_value_t = DEFAULT_FILE_DOWNLOAD_BYTES, value_parser = clap::value_parser!(u64).range(1..=MAX_FILE_DOWNLOAD_BYTES))]
        max_bytes: u64,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Upload one regular file to a root message or thread reply.
    Upload {
        /// Slack conversation ID or exact name.
        conversation: String,
        /// Local regular file with a UTF-8 basename of at most 255 bytes.
        #[arg(long)]
        path: std::path::PathBuf,
        /// Existing thread root timestamp. Omit to share at the conversation root.
        #[arg(long)]
        thread_ts: Option<String>,
        /// Optional Slack title: 1 to 255 UTF-8 bytes without control characters.
        #[arg(long)]
        title: Option<String>,
        /// Optional image alt text: 1 to 1,000 UTF-8 bytes without control characters.
        #[arg(long)]
        alt_text: Option<String>,
        /// Maximum source bytes to read, up to 1 GiB.
        #[arg(long, default_value_t = DEFAULT_FILE_UPLOAD_BYTES, value_parser = clap::value_parser!(u64).range(1..=MAX_FILE_UPLOAD_BYTES))]
        max_bytes: u64,
        /// Confirm the Slack mutation.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum EmojiCommand {
    /// List bounded workspace custom emoji and aliases.
    List {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReactionsCommand {
    /// Ensure an emoji reaction is present on one exact message.
    Add {
        /// Slack conversation ID or exact name.
        conversation: String,
        /// Exact Slack message timestamp.
        message_ts: String,
        /// Emoji name, with or without surrounding colons.
        name: String,
        /// Confirm the Slack mutation.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Ensure an emoji reaction is absent from one exact message.
    Remove {
        /// Slack conversation ID or exact name.
        conversation: String,
        /// Exact Slack message timestamp.
        message_ts: String,
        /// Emoji name, with or without surrounding colons.
        name: String,
        /// Confirm the Slack mutation.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Import a Chrome Copy-as-cURL request from standard input.
    ImportCurl {
        /// Allow an existing profile to change Slack workspace.
        #[arg(long)]
        replace_workspace: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List registered credential profiles without reading their secrets.
    List {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show non-secret metadata and credential presence for one profile.
    Status {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove one profile's credential file and registry entry.
    Remove {
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChannelCommand {
    /// Read recent channel history.
    Read {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        channel_id: String,
        /// Maximum messages to return, from 1 through 200.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Opaque Slack cursor from a previous channel response.
        #[arg(long)]
        cursor: Option<String>,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ThreadCommand {
    /// Read a thread root and its replies.
    Read {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        channel_id: String,
        /// Slack timestamp of the thread root.
        thread_ts: String,
        /// Maximum messages to return, from 1 through 200.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Opaque Slack cursor from a previous thread response.
        #[arg(long)]
        cursor: Option<String>,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reply to a thread with bounded Markdown from standard input.
    Reply {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        channel_id: String,
        /// Slack timestamp of the thread root.
        thread_ts: String,
        /// Also publish the reply to the conversation.
        #[arg(long)]
        broadcast: bool,
        /// Confirm irreversible message publication.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MessageCommand {
    /// Fetch one message by conversation ID or exact name and timestamp.
    Get {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        channel_id: String,
        /// Exact Slack message timestamp.
        message_ts: String,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render bounded Markdown from standard input as Slack rich text.
    Render {
        /// Emit the plain-text fallback and Slack blocks as stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Send a root message from bounded Markdown on standard input.
    Send {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        conversation: String,
        /// Confirm irreversible message publication.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DraftsCommand {
    /// List one timestamp-paginated page of active drafts.
    List {
        /// Private Slack draft timestamp from a previous response.
        #[arg(long)]
        next_ts: Option<String>,
        /// Maximum drafts to return, from 1 through 100.
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fetch one draft by its server ID.
    Get {
        /// Slack server draft ID.
        draft_id: String,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a draft from bounded Markdown on standard input.
    Create {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        conversation: String,
        /// Existing thread root timestamp. Omit for a root-message draft.
        #[arg(long)]
        thread_ts: Option<String>,
        /// Also send the eventual reply to the conversation. Requires --thread-ts.
        #[arg(long)]
        broadcast: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a one-file draft from bounded Markdown on standard input.
    CreateFile {
        /// Slack conversation ID or exact name; use # or @ to force a colliding name.
        conversation: String,
        /// Local regular file with a UTF-8 basename of at most 255 bytes.
        #[arg(long)]
        path: std::path::PathBuf,
        /// Existing thread root timestamp. Omit for a root-message draft.
        #[arg(long)]
        thread_ts: Option<String>,
        /// Also send the eventual reply to the conversation. Requires --thread-ts.
        #[arg(long)]
        broadcast: bool,
        /// Optional Slack title: 1 to 255 UTF-8 bytes without control characters.
        #[arg(long)]
        title: Option<String>,
        /// Optional image alt text: 1 to 1,000 UTF-8 bytes without control characters.
        #[arg(long)]
        alt_text: Option<String>,
        /// Maximum source bytes to read, up to 1 GiB.
        #[arg(long, default_value_t = DEFAULT_FILE_UPLOAD_BYTES, value_parser = clap::value_parser!(u64).range(1..=MAX_FILE_UPLOAD_BYTES))]
        max_bytes: u64,
        /// Confirm creation of the private Slack file and draft.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Replace a supported draft's content from bounded Markdown on standard input.
    Update {
        /// Slack server draft ID.
        draft_id: String,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Permanently delete one supported draft.
    Delete {
        /// Slack server draft ID.
        draft_id: String,
        /// Confirm permanent draft deletion.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Publish one supported draft, then delete it after Slack acknowledges the message.
    Send {
        /// Slack server draft ID.
        draft_id: String,
        /// Confirm irreversible message publication.
        #[arg(long)]
        confirm: bool,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConversationsCommand {
    /// List one cursor-paginated conversation page.
    List {
        /// Opaque Slack cursor from a previous list response.
        #[arg(long)]
        cursor: Option<String>,
        /// Maximum conversations to return, from 1 through 200.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Find conversations by ID, name, or DM participant name.
    Find {
        /// Case-insensitive substring to find.
        query: String,
        /// Maximum conversations to return, from 1 through 100.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum SearchCommand {
    /// Search messages newest first.
    Messages {
        /// Slack search text; standard Slack query modifiers are accepted.
        query: String,
        /// Restrict to an ID or exact name; use # or @ to force a colliding name.
        #[arg(long = "in")]
        conversation: Option<String>,
        /// Restrict to messages after this YYYY-MM-DD date.
        #[arg(long)]
        after: Option<String>,
        /// Restrict to messages before this YYYY-MM-DD date.
        #[arg(long)]
        before: Option<String>,
        /// Opaque Slack cursor from a previous search response.
        #[arg(long)]
        cursor: Option<String>,
        /// Maximum matching messages to return, from 1 through 100.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum UsersCommand {
    /// Find users by ID, handle, name, display name, or title.
    Find {
        /// Case-insensitive substring to find.
        query: String,
        /// Maximum users to return, from 1 through 100.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Auth { command } => run_auth(command, cli.profile.as_deref()).await,
        Command::Message {
            command: MessageCommand::Render { json },
        } => print_rendered(render_markdown(&read_markdown_stdin()?)?, json),
        command => run_slack_command(command, cli.profile.as_deref()).await,
    }
}

async fn run_auth(command: AuthCommand, profile: Option<&str>) -> Result<()> {
    match command {
        AuthCommand::ImportCurl {
            replace_workspace,
            json,
        } => {
            let profile = profile
                .ok_or_else(|| Error::invalid_input("profile", "is required for cURL import"))
                .and_then(ProfileName::parse)?;
            let bundle = {
                let input = read_curl_stdin()?;
                parse_copy_as_curl(&input)?
            };
            let config = Config::from_bundle_getter(bundle, |name| env::var(name).ok())?;
            let client = SlackHttpClient::new(config)?;
            let bundle = client.validate_session().await?.into_bundle();
            print_auth_import(store_profile(&profile, bundle, replace_workspace)?, json)
        }
        AuthCommand::List { json } => print_auth_list(list_profiles()?, json),
        AuthCommand::Status { json } => print_auth_status(profile_status(profile)?, json),
        AuthCommand::Remove { json } => print_auth_remove(remove_profile(profile)?, json),
    }
}

fn read_curl_stdin() -> Result<Zeroizing<Vec<u8>>> {
    let mut input = Zeroizing::new(Vec::new());
    std::io::stdin()
        .lock()
        .take((MAX_CURL_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| Error::InputRead)?;
    if input.len() > MAX_CURL_BYTES {
        return Err(Error::invalid_input("curl", "is larger than 256 KiB"));
    }
    Ok(input)
}

fn read_markdown_stdin() -> Result<String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take((MAX_MARKDOWN_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| Error::MarkdownInputRead)?;
    if input.len() > MAX_MARKDOWN_BYTES {
        return Err(Error::invalid_input(
            "markdown",
            "is larger than 40000 bytes",
        ));
    }
    String::from_utf8(input).map_err(|_| Error::invalid_input("markdown", "must be valid UTF-8"))
}

async fn run_slack_command(command: Command, profile: Option<&str>) -> Result<()> {
    let config = resolve_config(profile)?;
    let client = SlackHttpClient::new(config)?;
    let service = SlackService::new(client.clone(), client.config());
    match command {
        Command::Auth { .. } => unreachable!("authentication commands are dispatched first"),
        Command::Doctor { json } => print_doctor(service.doctor().await?, json),
        Command::Unreads { json } => print_unreads(service.unreads().await?, json),
        Command::Inbox {
            conversation_limit,
            message_limit,
            json,
        } => print_inbox(
            service.inbox(conversation_limit, message_limit).await?,
            json,
        ),
        Command::Conversations {
            command:
                ConversationsCommand::List {
                    cursor,
                    limit,
                    json,
                },
        } => print_conversation_page(
            service.list_conversations(cursor.as_deref(), limit).await?,
            json,
        ),
        Command::Conversations {
            command: ConversationsCommand::Find { query, limit, json },
        } => print_conversations(service.find_conversations(&query, limit).await?, json),
        Command::Search {
            command:
                SearchCommand::Messages {
                    query,
                    conversation,
                    after,
                    before,
                    cursor,
                    limit,
                    json,
                },
        } => print_message_search(
            service
                .search_messages(
                    &query,
                    conversation.as_deref(),
                    after.as_deref(),
                    before.as_deref(),
                    cursor.as_deref(),
                    limit,
                )
                .await?,
            json,
        ),
        Command::Channel {
            command:
                ChannelCommand::Read {
                    channel_id,
                    cursor,
                    limit,
                    json,
                },
        } => print_message_page(
            service
                .read_channel(&channel_id, cursor.as_deref(), limit)
                .await?,
            json,
        ),
        Command::Thread {
            command:
                ThreadCommand::Read {
                    channel_id,
                    thread_ts,
                    cursor,
                    limit,
                    json,
                },
        } => print_thread_page(
            service
                .read_thread(&channel_id, &thread_ts, cursor.as_deref(), limit)
                .await?,
            json,
        ),
        Command::Thread {
            command:
                ThreadCommand::Reply {
                    channel_id,
                    thread_ts,
                    broadcast,
                    confirm,
                    json,
                },
        } => {
            let markdown = read_markdown_stdin()?;
            print_sent_message(
                service
                    .send_message(&channel_id, Some(&thread_ts), broadcast, &markdown, confirm)
                    .await?,
                json,
            )
        }
        Command::Message {
            command:
                MessageCommand::Get {
                    channel_id,
                    message_ts,
                    json,
                },
        } => print_message(service.get_message(&channel_id, &message_ts).await?, json),
        Command::Message {
            command: MessageCommand::Render { .. },
        } => unreachable!("local Markdown rendering is dispatched before Slack configuration"),
        Command::Message {
            command:
                MessageCommand::Send {
                    conversation,
                    confirm,
                    json,
                },
        } => {
            let markdown = read_markdown_stdin()?;
            print_sent_message(
                service
                    .send_message(&conversation, None, false, &markdown, confirm)
                    .await?,
                json,
            )
        }
        Command::Drafts {
            command:
                DraftsCommand::List {
                    next_ts,
                    limit,
                    json,
                },
        } => print_draft_page(service.list_drafts(next_ts.as_deref(), limit).await?, json),
        Command::Drafts {
            command: DraftsCommand::Get { draft_id, json },
        } => print_draft(service.get_draft(&draft_id).await?, json),
        Command::Drafts {
            command:
                DraftsCommand::Create {
                    conversation,
                    thread_ts,
                    broadcast,
                    json,
                },
        } => {
            let markdown = read_markdown_stdin()?;
            print_draft(
                service
                    .create_draft(&conversation, thread_ts.as_deref(), broadcast, &markdown)
                    .await?,
                json,
            )
        }
        Command::Drafts {
            command:
                DraftsCommand::CreateFile {
                    conversation,
                    path,
                    thread_ts,
                    broadcast,
                    title,
                    alt_text,
                    max_bytes,
                    confirm,
                    json,
                },
        } => {
            if !confirm {
                return Err(Error::ConfirmationRequired {
                    action: "file draft creation",
                });
            }
            let file_name = validate_cli_upload_path(&path)?;
            SlackService::validate_file_draft_request(
                &conversation,
                thread_ts.as_deref(),
                broadcast,
                title.as_deref(),
                alt_text.as_deref(),
                &file_name,
            )?;
            let markdown = read_markdown_stdin()?;
            render_markdown(&markdown)?;
            let source = prepare_cli_upload(&path, max_bytes)?;
            print_file_draft_create(
                service
                    .create_file_draft(
                        FileDraftCreateRequest {
                            conversation: &conversation,
                            thread_ts: thread_ts.as_deref(),
                            broadcast,
                            markdown: &markdown,
                            title: title.as_deref(),
                            alt_text: alt_text.as_deref(),
                            confirmed: confirm,
                        },
                        source,
                    )
                    .await?,
                json,
            )
        }
        Command::Drafts {
            command: DraftsCommand::Update { draft_id, json },
        } => {
            let markdown = read_markdown_stdin()?;
            print_draft(service.update_draft(&draft_id, &markdown).await?, json)
        }
        Command::Drafts {
            command:
                DraftsCommand::Delete {
                    draft_id,
                    confirm,
                    json,
                },
        } => print_draft_delete(service.delete_draft(&draft_id, confirm).await?, json),
        Command::Drafts {
            command:
                DraftsCommand::Send {
                    draft_id,
                    confirm,
                    json,
                },
        } => print_draft_send(service.send_draft(&draft_id, confirm).await?, json),
        Command::Files {
            command: FilesCommand::Info { file_id, json },
        } => print_file(service.get_file(&file_id).await?, json),
        Command::Files {
            command:
                FilesCommand::Download {
                    file_id,
                    output,
                    max_bytes,
                    json,
                },
        } => {
            validate_cli_download_path(&output)?;
            let file = service.get_file(&file_id).await?;
            let size = file.size.ok_or(Error::NotFound {
                resource: "Slack file size",
            })?;
            if size > max_bytes {
                return Err(Error::invalid_input(
                    "max_bytes",
                    "is smaller than the Slack file size",
                ));
            }
            let target = prepare_cli_download(&output, max_bytes)?;
            let output_path = output.to_string_lossy().into_owned();
            print_file_download(
                service.download_file(file, target, output_path).await?,
                json,
            )
        }
        Command::Files {
            command:
                FilesCommand::Upload {
                    conversation,
                    path,
                    thread_ts,
                    title,
                    alt_text,
                    max_bytes,
                    confirm,
                    json,
                },
        } => {
            if !confirm {
                return Err(Error::ConfirmationRequired {
                    action: "file upload",
                });
            }
            let file_name = validate_cli_upload_path(&path)?;
            SlackService::validate_upload_request(
                &conversation,
                thread_ts.as_deref(),
                title.as_deref(),
                alt_text.as_deref(),
                &file_name,
            )?;
            let source = prepare_cli_upload(&path, max_bytes)?;
            print_file_upload(
                service
                    .upload_file(
                        &conversation,
                        thread_ts.as_deref(),
                        title.as_deref(),
                        alt_text.as_deref(),
                        source,
                        confirm,
                    )
                    .await?,
                json,
            )
        }
        Command::Emoji {
            command: EmojiCommand::List { json },
        } => print_custom_emoji(service.list_custom_emoji().await?, json),
        Command::Reactions {
            command:
                ReactionsCommand::Add {
                    conversation,
                    message_ts,
                    name,
                    confirm,
                    json,
                },
        } => print_reaction_mutation(
            service
                .add_reaction(&conversation, &message_ts, &name, confirm)
                .await?,
            json,
        ),
        Command::Reactions {
            command:
                ReactionsCommand::Remove {
                    conversation,
                    message_ts,
                    name,
                    confirm,
                    json,
                },
        } => print_reaction_mutation(
            service
                .remove_reaction(&conversation, &message_ts, &name, confirm)
                .await?,
            json,
        ),
        Command::Users {
            command: UsersCommand::Find { query, limit, json },
        } => print_users(service.find_users(&query, limit).await?, json),
        Command::Mcp {
            allow_write,
            file_root,
        } => mcp::serve_stdio(service, allow_write, file_root).await,
    }
}

fn print_file(file: FileReference, json: bool) -> Result<()> {
    if json {
        print_json(&file)
    } else {
        println!("{}", format_file(&file));
        Ok(())
    }
}

fn format_file(file: &FileReference) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        escape_human(&file.id),
        file.size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "-".into()),
        escape_human(file.mimetype.as_deref().unwrap_or("-")),
        escape_human(file.name.as_deref().unwrap_or("-")),
        escape_human(file.download_url.as_deref().unwrap_or("-"))
    )
}

fn print_file_download(report: FileDownloadReport, json: bool) -> Result<()> {
    if json {
        print_json(&report)
    } else {
        println!("{}", format_file_download(&report));
        if let Some(warning) = &report.durability_warning {
            eprintln!("warning: {}", escape_human(warning));
        }
        Ok(())
    }
}

fn format_file_download(report: &FileDownloadReport) -> String {
    format!(
        "downloaded\t{}\t{}\t{}",
        escape_human(&report.file.id),
        report.bytes_written,
        escape_human(&report.output_path)
    )
}

fn print_file_upload(report: FileUploadReport, json: bool) -> Result<()> {
    if json {
        print_json(&report)
    } else {
        println!("{}", format_file_upload(&report));
        Ok(())
    }
}

fn print_file_draft_create(report: FileDraftCreateReport, json: bool) -> Result<()> {
    if json {
        print_json(&report)
    } else {
        println!("{}", format_file_draft_create(&report));
        Ok(())
    }
}

fn format_file_draft_create(report: &FileDraftCreateReport) -> String {
    match report {
        FileDraftCreateReport::AllocationUncertain => "allocation_uncertain".into(),
        FileDraftCreateReport::Allocated { file_id } => {
            format!("allocated\t{}", escape_human(file_id))
        }
        FileDraftCreateReport::SourceChanged { file_id } => {
            format!("source_changed\t{}", escape_human(file_id))
        }
        FileDraftCreateReport::TransferUncertain { file_id } => {
            format!("transfer_uncertain\t{}", escape_human(file_id))
        }
        FileDraftCreateReport::FileCompletionUncertain { file_id } => {
            format!("file_completion_uncertain\t{}", escape_human(file_id))
        }
        FileDraftCreateReport::DraftNotCreated { file_id, reason } => format!(
            "draft_not_created\t{}\t{}",
            escape_human(file_id),
            escape_human(reason)
        ),
        FileDraftCreateReport::DraftCreationUncertain {
            file_id,
            client_msg_id,
        } => format!(
            "draft_creation_uncertain\t{}\t{}",
            escape_human(file_id),
            escape_human(client_msg_id)
        ),
        FileDraftCreateReport::Created {
            draft,
            file,
            reconciled,
        } => format!(
            "created\t{}\t{}\treconciled={}",
            escape_human(&draft.id),
            escape_human(&file.id),
            reconciled
        ),
    }
}

fn format_file_upload(report: &FileUploadReport) -> String {
    match report {
        FileUploadReport::AllocationUncertain => "allocation_uncertain".into(),
        FileUploadReport::Allocated { file_id } => {
            format!("allocated\t{}", escape_human(file_id))
        }
        FileUploadReport::SourceChanged { file_id } => {
            format!("source_changed\t{}", escape_human(file_id))
        }
        FileUploadReport::TransferUncertain { file_id } => {
            format!("transfer_uncertain\t{}", escape_human(file_id))
        }
        FileUploadReport::CompletionUncertain { file_id } => {
            format!("completion_uncertain\t{}", escape_human(file_id))
        }
        FileUploadReport::Shared {
            file,
            share,
            reconciled,
        } => format!(
            "shared\t{}\t{}\t{}\t{}\treconciled={}",
            escape_human(&file.id),
            escape_human(&share.channel_id),
            escape_human(&share.ts),
            escape_human(share.thread_ts.as_deref().unwrap_or("-")),
            reconciled
        ),
    }
}

fn print_custom_emoji(list: CustomEmojiList, json: bool) -> Result<()> {
    if json {
        print_json(&list)
    } else {
        let output = format_custom_emoji(&list);
        if !output.is_empty() {
            println!("{output}");
        }
        Ok(())
    }
}

fn format_custom_emoji(list: &CustomEmojiList) -> String {
    list.emoji
        .iter()
        .map(|emoji| {
            let kind = match emoji.kind {
                CustomEmojiKind::Image => "image",
                CustomEmojiKind::Alias => "alias",
            };
            let target = emoji
                .alias_for
                .as_deref()
                .or(emoji.image_url.as_deref())
                .unwrap_or("-");
            format!(
                "{}\t{}\t{}",
                escape_human(&emoji.name),
                kind,
                escape_human(target)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_reaction_mutation(report: ReactionMutationReport, json: bool) -> Result<()> {
    if json {
        print_json(&report)
    } else {
        println!("{}", format_reaction_mutation(&report));
        Ok(())
    }
}

fn format_reaction_mutation(report: &ReactionMutationReport) -> String {
    format!(
        "{}\t{}\t{}\t{}\ttarget_present={}\tchanged={}\treconciled={}",
        if report.present { "present" } else { "absent" },
        escape_human(&report.channel_id),
        escape_human(&report.message_ts),
        escape_human(&report.name),
        report.target_present,
        report.changed,
        report.reconciled
    )
}

fn print_rendered(rendered: RenderedMessage, json: bool) -> Result<()> {
    if json {
        print_json(&rendered)
    } else {
        println!("{}", rendered.text);
        Ok(())
    }
}

fn print_draft_page(page: DraftPage, json: bool) -> Result<()> {
    if json {
        return print_json(&page);
    }
    if page.drafts.is_empty() {
        println!("No active drafts.");
    } else {
        for draft in &page.drafts {
            print_draft_row(draft);
        }
    }
    if page.has_more {
        println!("more\t{}", page.next_ts.as_deref().unwrap_or("available"));
    }
    Ok(())
}

fn print_draft(draft: Draft, json: bool) -> Result<()> {
    if json {
        print_json(&draft)
    } else {
        print_draft_row(&draft);
        Ok(())
    }
}

fn print_draft_row(draft: &Draft) {
    let destination = draft.destinations.first();
    println!(
        "{}\t{}\t{}\t{}\t{}",
        escape_human(&draft.id),
        escape_human(
            destination
                .and_then(|destination| destination.channel_id.as_deref())
                .unwrap_or("-")
        ),
        escape_human(
            destination
                .and_then(|destination| destination.thread_ts.as_deref())
                .unwrap_or("root")
        ),
        match draft.file_association {
            Some(FileDraftAssociation::Unverified) => "file-unverified",
            Some(FileDraftAssociation::Verified) | None if draft.is_supported => "supported",
            _ => "unsupported",
        },
        escape_human(&draft.text)
    );
}

fn print_draft_delete(report: DraftDeleteReport, json: bool) -> Result<()> {
    if json {
        print_json(&report)
    } else {
        println!("{}", format_draft_delete(&report));
        Ok(())
    }
}

fn format_draft_delete(report: &DraftDeleteReport) -> String {
    if let Some(file_id) = &report.file_id {
        format!(
            "deleted\t{}\tfile-preserved\t{}",
            escape_human(&report.id),
            escape_human(file_id)
        )
    } else {
        format!("deleted\t{}", escape_human(&report.id))
    }
}

fn print_sent_message(sent: SentMessage, json: bool) -> Result<()> {
    if json {
        print_json(&sent)
    } else {
        println!(
            "sent\t{}\t{}\t{}",
            escape_human(&sent.message.channel_id),
            escape_human(&sent.message.ts),
            escape_human(&sent.client_msg_id)
        );
        Ok(())
    }
}

fn print_draft_send(report: DraftSendReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    println!(
        "sent\t{}\t{}\tdraft={}",
        escape_human(&report.sent.message.channel_id),
        escape_human(&report.sent.message.ts),
        escape_human(&report.draft_id)
    );
    if let Some(warning) = report.cleanup_warning {
        eprintln!(
            "warning: message was sent, but deletion of draft {} at revision {} was not confirmed: {}",
            escape_human(&warning.draft_id),
            escape_human(&warning.last_updated_ts),
            escape_human(&warning.reason)
        );
    }
    Ok(())
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|_| Error::Output)?
    );
    Ok(())
}

fn print_auth_import(report: AuthImportReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    println!(
        "stored\t{}\nworkspace\t{}\nteam\t{}\ndefault\t{}\nreplaced_workspace\t{}",
        report.profile,
        report.workspace_url,
        report.team_id,
        report.default,
        report.replaced_workspace
    );
    Ok(())
}

fn print_auth_list(report: AuthListReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    if report.profiles.is_empty() {
        println!("No credential profiles.");
        return Ok(());
    }
    for profile in report.profiles {
        let marker = if profile.default { "*" } else { " " };
        println!(
            "{marker}\t{}\t{}\tteam={}",
            profile.profile, profile.workspace_url, profile.team_id
        );
    }
    Ok(())
}

fn print_auth_status(report: AuthStatusReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    println!(
        "profile\t{}\nworkspace\t{}\nteam\t{}\ndefault\t{}\ncredential_present\t{}",
        report.profile,
        report.workspace_url,
        report.team_id,
        report.default,
        report.credential_present
    );
    Ok(())
}

fn print_auth_remove(report: AuthRemoveReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    println!(
        "removed\t{}\ndefault\t{}",
        report.profile,
        report.default_profile.as_deref().unwrap_or("-")
    );
    Ok(())
}

fn print_doctor(report: DoctorReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    println!(
        "authenticated\t{}\nteam\t{}\nworkspace\t{}",
        report.authenticated, report.team_id, report.workspace_url
    );
    Ok(())
}

fn print_unreads(report: UnreadReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    if report.conversations.is_empty() && !report.threads.has_unreads {
        println!("No unread conversations or threads.");
        return Ok(());
    }
    for conversation in report.conversations {
        let kind = conversation_kind_label(conversation.kind);
        println!(
            "{kind}\t{}\tmentions={}\tlast_read={}\tlatest={}",
            conversation.id,
            conversation.mention_count,
            escape_human(conversation.last_read.as_deref().unwrap_or("-")),
            escape_human(conversation.latest.as_deref().unwrap_or("-"))
        );
    }
    if report.threads.has_unreads {
        println!(
            "threads\tchannels={}\tmentions={}",
            report.threads.unread_count_by_channel.len(),
            report.threads.mention_count
        );
    }
    Ok(())
}

fn print_inbox(report: InboxReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    if inbox_is_clear(&report) {
        println!("Inbox is clear.");
        return Ok(());
    }
    let more_conversations = inbox_more_conversations_line(&report);
    for entry in report.conversations {
        println!(
            "{}\t{}\t{}\tmentions={}",
            conversation_kind_label(entry.conversation.kind),
            entry.conversation.id,
            escape_human(&entry.conversation.display_name),
            entry.unread.mention_count
        );
        print_messages(&entry.messages.messages);
        if entry.messages.has_more {
            println!(
                "more\t{}\t{}",
                entry.conversation.id,
                entry.messages.next_cursor.as_deref().unwrap_or("available")
            );
        }
    }
    if let Some(line) = more_conversations {
        println!("{line}");
    }
    if report.threads.has_unreads {
        println!(
            "threads\tchannels={}\tmentions={}",
            report.threads.unread_count_by_channel.len(),
            report.threads.mention_count
        );
    }
    Ok(())
}

fn inbox_is_clear(report: &InboxReport) -> bool {
    report.conversations.is_empty() && !report.threads.has_unreads && !report.has_more_conversations
}

fn inbox_more_conversations_line(report: &InboxReport) -> Option<String> {
    if !report.has_more_conversations {
        return None;
    }
    let reason = match report.truncation_reason {
        Some(InboxTruncationReason::ConversationLimit) => "conversation-limit",
        Some(InboxTruncationReason::ByteLimit) => "byte-limit",
        None => "unknown",
    };
    Some(format!(
        "more-conversations\tshown={}\ttotal={}\treason={}",
        report.conversations.len(),
        report.total_unread_conversations,
        reason
    ))
}

fn print_conversation_page(page: ConversationPage, json: bool) -> Result<()> {
    if json {
        return print_json(&page);
    }
    print_conversation_rows(&page.conversations);
    if page.has_more {
        println!(
            "more\t{}",
            page.next_cursor.as_deref().unwrap_or("available")
        );
    }
    Ok(())
}

fn print_conversations(report: ConversationSearchReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    print_conversation_rows(&report.conversations);
    if let Some(notice) = conversation_truncation_notice(&report) {
        println!("{notice}");
    }
    Ok(())
}

fn print_conversation_rows(conversations: &[Conversation]) {
    if conversations.is_empty() {
        println!("No conversations matched.");
        return;
    }
    for conversation in conversations {
        let prefix = match conversation.kind {
            ConversationKind::Channel => "#",
            ConversationKind::DirectMessage => "@",
            ConversationKind::GroupDirectMessage => "",
        };
        println!(
            "{}\t{}\t{}{}\t{}",
            conversation_kind_label(conversation.kind),
            conversation.id,
            prefix,
            escape_human(&conversation.name),
            escape_human(&conversation.display_name)
        );
    }
}

fn conversation_kind_label(kind: ConversationKind) -> &'static str {
    match kind {
        ConversationKind::Channel => "channel",
        ConversationKind::DirectMessage => "dm",
        ConversationKind::GroupDirectMessage => "group-dm",
    }
}

fn conversation_truncation_notice(report: &ConversationSearchReport) -> Option<String> {
    match report.truncation_reason {
        Some(ConversationSearchTruncationReason::ResultLimit)
            if report.conversations.len() < MAX_CONVERSATIONS =>
        {
            Some(format!(
                "Result limit reached after scanning {} conversations; raise --limit to return more matches.",
                report.scanned_conversations
            ))
        }
        Some(ConversationSearchTruncationReason::ResultLimit) => Some(format!(
            "More matches exist after scanning {} conversations; the {}-result maximum was reached.",
            report.scanned_conversations, MAX_CONVERSATIONS
        )),
        Some(ConversationSearchTruncationReason::ScanLimit) => Some(format!(
            "Search stopped after scanning {} conversations (scan cap {}); matches may exist beyond the scanned pages.",
            report.scanned_conversations, report.scan_limit
        )),
        None => None,
    }
}

fn print_message_search(page: MessageSearchPage, json: bool) -> Result<()> {
    if json {
        return print_json(&page);
    }
    if page.matches.is_empty() {
        println!("No messages matched.");
    } else {
        for message in &page.matches {
            println!("{}", format_search_match(message));
        }
    }
    println!("total\t{}", page.total);
    if page.has_more {
        println!(
            "more\t{}",
            page.next_cursor.as_deref().unwrap_or("available")
        );
    }
    Ok(())
}

fn print_message_page(page: MessagePage, json: bool) -> Result<()> {
    if json {
        return print_json(&page);
    }
    print_messages(&page.messages);
    if page.has_more {
        println!(
            "more\t{}",
            page.next_cursor.as_deref().unwrap_or("available")
        );
    }
    Ok(())
}

fn print_thread_page(page: ThreadPage, json: bool) -> Result<()> {
    if json {
        return print_json(&page);
    }
    println!("thread\t{}\t{}", page.channel_id, page.thread_ts);
    print_messages(&page.messages);
    if page.has_more {
        println!(
            "more\t{}",
            page.next_cursor.as_deref().unwrap_or("available")
        );
    }
    Ok(())
}

fn print_message(message: Message, json: bool) -> Result<()> {
    if json {
        return print_json(&message);
    }
    print_messages(&[message]);
    Ok(())
}

fn print_messages(messages: &[Message]) {
    if messages.is_empty() {
        println!("No messages.");
        return;
    }
    for message in messages {
        let author = escape_human(
            message
                .author_id
                .as_deref()
                .or(message.author_name.as_deref())
                .unwrap_or("-"),
        );
        let text = escape_human(&message.text);
        println!(
            "{}\t{}\t{}\treplies={}",
            message.ts, author, text, message.reply_count
        );
    }
}

fn print_users(report: UserSearchReport, json: bool) -> Result<()> {
    if json {
        return print_json(&report);
    }
    if report.users.is_empty() {
        println!("No users matched.");
    } else {
        for user in &report.users {
            println!(
                "{}\t@{}\t{}\t{}",
                user.id,
                escape_human(&user.name),
                escape_human(&user.display_name),
                escape_human(&user.real_name)
            );
        }
    }
    if let Some(notice) = user_truncation_notice(&report) {
        println!("{notice}");
    }
    Ok(())
}

fn format_search_match(message: &crate::model::MessageSearchMatch) -> String {
    let author = message
        .author_id
        .as_deref()
        .or(message.author_name.as_deref())
        .unwrap_or("-");
    format!(
        "{}\t{}\t{}\t{}\t{}",
        escape_human(&message.ts),
        escape_human(&message.channel_id),
        escape_human(&message.channel_name),
        escape_human(author),
        escape_human(&message.text)
    )
}

fn escape_human(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                write!(&mut escaped, "\\u{{{:x}}}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn user_truncation_notice(report: &UserSearchReport) -> Option<String> {
    match report.truncation_reason {
        Some(UserSearchTruncationReason::ResultLimit) if report.users.len() < MAX_USERS => {
            Some(format!(
                "Result limit reached after scanning {} users; raise --limit to return more matches.",
                report.scanned_users
            ))
        }
        Some(UserSearchTruncationReason::ResultLimit) => Some(format!(
            "More matches exist after scanning {} users; the {}-result maximum was reached.",
            report.scanned_users, MAX_USERS
        )),
        Some(UserSearchTruncationReason::ScanLimit) => Some(format!(
            "Search stopped after scanning {} users (scan cap {}); matches may exist beyond the scanned pages.",
            report.scanned_users, report.scan_limit
        )),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_milestone_one_commands() {
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "doctor", "--json"])
                .unwrap()
                .command,
            Command::Doctor { json: true }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "unreads"])
                .unwrap()
                .command,
            Command::Unreads { json: false }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "inbox",
                "--conversations",
                "4",
                "--messages",
                "8",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Inbox {
                conversation_limit: 4,
                message_limit: 8,
                json: true
            }
        ));
    }

    #[test]
    fn parses_auth_commands_with_profile_at_any_depth() {
        let cli = Cli::try_parse_from([
            "lurkline",
            "auth",
            "import-curl",
            "--replace-workspace",
            "--json",
            "--profile",
            "sferait",
        ])
        .unwrap();
        assert_eq!(cli.profile.as_deref(), Some("sferait"));
        assert!(matches!(
            cli.command,
            Command::Auth {
                command: AuthCommand::ImportCurl {
                    replace_workspace: true,
                    json: true
                }
            }
        ));

        for command in ["list", "status", "remove"] {
            let cli =
                Cli::try_parse_from(["lurkline", "--profile", "work", "auth", command, "--json"])
                    .unwrap();
            assert_eq!(cli.profile.as_deref(), Some("work"));
            assert!(matches!(cli.command, Command::Auth { .. }));
        }
    }

    #[test]
    fn parses_all_read_commands_and_bounds_flags() {
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "conversations",
                "list",
                "--cursor",
                "next-page",
                "--limit",
                "12",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Conversations {
                command: ConversationsCommand::List {
                    cursor: Some(_),
                    limit: 12,
                    json: true
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "conversations", "find", "general"])
                .unwrap()
                .command,
            Command::Conversations {
                command: ConversationsCommand::Find { limit: 20, .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "search",
                "messages",
                "deploy",
                "--in",
                "general",
                "--after",
                "2026-01-01",
                "--before",
                "2026-02-01",
                "--cursor",
                "next",
                "--limit",
                "12",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Search {
                command: SearchCommand::Messages {
                    conversation: Some(_),
                    after: Some(_),
                    before: Some(_),
                    cursor: Some(_),
                    limit: 12,
                    json: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "channel",
                "read",
                "C123",
                "--cursor",
                "channel-next",
                "--limit",
                "12",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Channel {
                command: ChannelCommand::Read {
                    cursor: Some(_),
                    limit: 12,
                    json: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "thread",
                "read",
                "C123",
                "100.000001",
                "--cursor",
                "thread-next"
            ])
            .unwrap()
            .command,
            Command::Thread {
                command: ThreadCommand::Read {
                    cursor: Some(_),
                    limit: 100,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "message", "get", "C123", "100.000001"])
                .unwrap()
                .command,
            Command::Message { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "message", "render", "--json"])
                .unwrap()
                .command,
            Command::Message {
                command: MessageCommand::Render { json: true }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "users", "find", "alice", "--limit", "3"])
                .unwrap()
                .command,
            Command::Users {
                command: UsersCommand::Find { limit: 3, .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "mcp", "--allow-write"])
                .unwrap()
                .command,
            Command::Mcp {
                allow_write: true,
                ..
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "drafts",
                "create",
                "#general",
                "--thread-ts",
                "100.000001",
                "--broadcast",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Drafts {
                command: DraftsCommand::Create {
                    thread_ts: Some(_),
                    broadcast: true,
                    json: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn parses_guarded_root_reply_and_draft_publication() {
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "message",
                "send",
                "#general",
                "--confirm",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Message {
                command: MessageCommand::Send {
                    conversation,
                    confirm: true,
                    json: true
                }
            } if conversation == "#general"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "thread",
                "reply",
                "C123",
                "100.000001",
                "--broadcast",
                "--confirm"
            ])
            .unwrap()
            .command,
            Command::Thread {
                command: ThreadCommand::Reply {
                    broadcast: true,
                    confirm: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "drafts", "send", "DR123", "--confirm"])
                .unwrap()
                .command,
            Command::Drafts {
                command: DraftsCommand::Send { confirm: true, .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "drafts",
                "create-file",
                "@smarzola",
                "--path",
                "synthetic.txt",
                "--thread-ts",
                "100.000001",
                "--broadcast",
                "--title",
                "Synthetic",
                "--alt-text",
                "Synthetic test file",
                "--max-bytes",
                "1024",
                "--confirm",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Drafts {
                command: DraftsCommand::CreateFile {
                    max_bytes: 1024,
                    confirm: true,
                    json: true,
                    broadcast: true,
                    thread_ts: Some(_),
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lurkline",
                "files",
                "upload",
                "@smarzola",
                "--path",
                "synthetic.txt",
                "--thread-ts",
                "100.000001",
                "--title",
                "Synthetic",
                "--alt-text",
                "Synthetic test file",
                "--max-bytes",
                "1024",
                "--confirm",
                "--json"
            ])
            .unwrap()
            .command,
            Command::Files {
                command: FilesCommand::Upload {
                    max_bytes: 1024,
                    confirm: true,
                    json: true,
                    thread_ts: Some(_),
                    ..
                }
            }
        ));
    }

    #[test]
    fn warns_when_an_empty_user_search_hit_the_scan_cap() {
        let report = UserSearchReport {
            query: "missing".into(),
            users: vec![],
            truncated: true,
            truncation_reason: Some(UserSearchTruncationReason::ScanLimit),
            scanned_users: 4_000,
            scan_limit: 4_000,
        };
        assert_eq!(
            user_truncation_notice(&report).as_deref(),
            Some(
                "Search stopped after scanning 4000 users (scan cap 4000); matches may exist beyond the scanned pages."
            )
        );
    }

    #[test]
    fn does_not_suggest_an_impossible_limit_above_the_maximum() {
        let report = UserSearchReport {
            query: "many".into(),
            users: (0..MAX_USERS)
                .map(|index| crate::model::User {
                    id: format!("U{index}"),
                    name: String::new(),
                    display_name: String::new(),
                    real_name: String::new(),
                    title: String::new(),
                    deleted: false,
                    is_bot: false,
                    timezone: None,
                    image_url: None,
                })
                .collect(),
            truncated: true,
            truncation_reason: Some(UserSearchTruncationReason::ResultLimit),
            scanned_users: 101,
            scan_limit: 4_000,
        };
        let notice = user_truncation_notice(&report).unwrap();
        assert!(notice.contains("100-result maximum"));
        assert!(!notice.contains("raise --limit"));
    }

    #[test]
    fn escapes_every_control_character_in_human_output() {
        assert_eq!(
            escape_human("plain\r\x1b\n\t\u{8}text"),
            "plain\\r\\u{1b}\\n\\t\\u{8}text"
        );
        assert_eq!(escape_human("café 🚀"), "café 🚀");
    }

    #[test]
    fn search_locations_render_ids_and_names_without_assuming_channel_kind() {
        for (id, name) in [
            ("C123", "general"),
            ("D123", "U123"),
            ("G123", "mpdm-alice--bob-1"),
        ] {
            let message = crate::model::MessageSearchMatch {
                channel_id: id.into(),
                channel_name: name.into(),
                ts: "100.000001".into(),
                thread_ts: None,
                author_id: Some("U456".into()),
                author_name: None,
                text: "hello\r\x1b".into(),
                blocks: None,
                attachments: None,
                reactions: vec![],
                files: vec![],
                permalink: None,
            };
            assert_eq!(
                format_search_match(&message),
                format!("100.000001\t{id}\t{name}\tU456\thello\\r\\u{{1b}}")
            );
        }
    }

    fn synthetic_file() -> FileReference {
        FileReference {
            id: "F123".into(),
            name: Some("report\tfinal.pdf".into()),
            title: Some("Report".into()),
            alt_text: None,
            mimetype: Some("application/pdf".into()),
            filetype: Some("pdf".into()),
            pretty_type: Some("PDF".into()),
            mode: Some("hosted".into()),
            file_access: None,
            uploader_id: Some("U123".into()),
            size: Some(42),
            created: Some(1),
            timestamp: Some(2),
            editable: Some(false),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            private_url: Some("https://files.slack.com/private".into()),
            download_url: Some("https://files.slack.com/file\nname".into()),
            permalink: Some("https://workspace.slack.com/files/F123".into()),
            channel_ids: None,
            group_ids: None,
            im_ids: None,
            shares: Some(vec![]),
            shares_complete: true,
        }
    }

    #[test]
    fn new_human_printers_are_stable_and_escape_untrusted_fields() {
        let file = synthetic_file();
        assert_eq!(
            format_file(&file),
            "F123\t42\tapplication/pdf\treport\\tfinal.pdf\thttps://files.slack.com/file\\nname"
        );

        let download = FileDownloadReport {
            file: file.clone(),
            output_path: "downloads/report\nfinal.pdf".into(),
            bytes_written: 42,
            durability_warning: None,
        };
        assert_eq!(
            format_file_download(&download),
            "downloaded\tF123\t42\tdownloads/report\\nfinal.pdf"
        );

        let upload = FileUploadReport::Shared {
            file: Box::new(file.clone()),
            share: crate::model::FileShare {
                visibility: crate::model::FileShareVisibility::Private,
                channel_id: "D123".into(),
                ts: "100.000001".into(),
                thread_ts: Some("90.000001".into()),
            },
            reconciled: true,
        };
        assert_eq!(
            format_file_upload(&upload),
            "shared\tF123\tD123\t100.000001\t90.000001\treconciled=true"
        );
        assert_eq!(
            format_file_upload(&FileUploadReport::TransferUncertain {
                file_id: "F123".into()
            }),
            "transfer_uncertain\tF123"
        );
        assert_eq!(
            format_file_draft_create(&FileDraftCreateReport::DraftNotCreated {
                file_id: "F123".into(),
                reason: "safe\nreason".into(),
            }),
            "draft_not_created\tF123\tsafe\\nreason"
        );

        let emoji = CustomEmojiList {
            emoji: vec![
                crate::model::CustomEmoji {
                    name: "party\tparrot".into(),
                    kind: CustomEmojiKind::Image,
                    image_url: Some("https://emoji.slack-edge.com/image\nurl".into()),
                    alias_for: None,
                },
                crate::model::CustomEmoji {
                    name: "shipit".into(),
                    kind: CustomEmojiKind::Alias,
                    image_url: None,
                    alias_for: Some("rocket".into()),
                },
            ],
        };
        assert_eq!(
            format_custom_emoji(&emoji),
            "party\\tparrot\timage\thttps://emoji.slack-edge.com/image\\nurl\nshipit\talias\trocket"
        );

        let reaction = ReactionMutationReport {
            channel_id: "C123".into(),
            message_ts: "100.000001".into(),
            name: "eyes\nwide".into(),
            target_present: true,
            present: true,
            changed: false,
            reconciled: true,
        };
        assert_eq!(
            format_reaction_mutation(&reaction),
            "present\tC123\t100.000001\teyes\\nwide\ttarget_present=true\tchanged=false\treconciled=true"
        );
        assert_eq!(
            format_draft_delete(&DraftDeleteReport {
                id: "DR123".into(),
                deleted: true,
                file_id: Some("F123".into()),
                file_deleted: Some(false),
            }),
            "deleted\tDR123\tfile-preserved\tF123"
        );
        assert_eq!(
            Error::DraftCreationUncertain {
                client_msg_id: "00000000-0000-4000-8000-000000000001".into(),
            }
            .to_string(),
            "Slack draft creation outcome is unknown for client message 00000000-0000-4000-8000-000000000001; do not retry automatically; reread active drafts before deciding whether to retry"
        );
    }

    #[test]
    fn new_json_models_have_stable_field_names() {
        let file = serde_json::to_value(synthetic_file()).unwrap();
        assert_eq!(file["id"], "F123");
        assert_eq!(file["size"], 42);
        assert_eq!(file["channel_ids"], serde_json::Value::Null);
        assert_eq!(file["group_ids"], serde_json::Value::Null);
        assert_eq!(file["im_ids"], serde_json::Value::Null);
        assert_eq!(file["shares"], serde_json::json!([]));
        assert_eq!(file["shares_complete"], true);

        assert_eq!(
            serde_json::to_value(FileUploadReport::Allocated {
                file_id: "F123".into()
            })
            .unwrap(),
            serde_json::json!({
                "stage": "allocated",
                "file_id": "F123"
            })
        );
        assert_eq!(
            serde_json::to_value(FileUploadReport::AllocationUncertain).unwrap(),
            serde_json::json!({
                "stage": "allocation_uncertain"
            })
        );
        assert_eq!(
            serde_json::to_value(FileDraftCreateReport::DraftCreationUncertain {
                file_id: "F123".into(),
                client_msg_id: "00000000-0000-4000-8000-000000000001".into(),
            })
            .unwrap(),
            serde_json::json!({
                "stage": "draft_creation_uncertain",
                "file_id": "F123",
                "client_msg_id": "00000000-0000-4000-8000-000000000001"
            })
        );

        let reaction = serde_json::to_value(ReactionMutationReport {
            channel_id: "C123".into(),
            message_ts: "100.000001".into(),
            name: "eyes".into(),
            target_present: false,
            present: false,
            changed: true,
            reconciled: false,
        })
        .unwrap();
        assert_eq!(
            reaction,
            serde_json::json!({
                "channel_id": "C123",
                "message_ts": "100.000001",
                "name": "eyes",
                "target_present": false,
                "present": false,
                "changed": true,
                "reconciled": false
            })
        );
    }

    #[test]
    fn empty_byte_truncated_inbox_is_not_rendered_as_clear() {
        let report = InboxReport {
            team_id: "T000TEST".into(),
            conversations: Vec::new(),
            total_unread_conversations: 1,
            has_more_conversations: true,
            truncation_reason: Some(InboxTruncationReason::ByteLimit),
            threads: crate::model::UnreadThreads {
                has_unreads: false,
                mention_count: 0,
                unread_count_by_channel: std::collections::BTreeMap::new(),
            },
        };

        assert_eq!(
            inbox_more_conversations_line(&report).as_deref(),
            Some("more-conversations\tshown=0\ttotal=1\treason=byte-limit")
        );
        assert!(!inbox_is_clear(&report));
    }
}
