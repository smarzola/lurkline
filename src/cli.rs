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
    markdown::{MAX_MARKDOWN_BYTES, render_markdown},
    mcp,
    model::{
        Conversation, ConversationKind, ConversationPage, ConversationSearchReport,
        ConversationSearchTruncationReason, DoctorReport, InboxReport, Message, MessagePage,
        MessageSearchPage, RenderedMessage, ThreadPage, UnreadReport, UserSearchReport,
        UserSearchTruncationReason,
    },
    service::{MAX_CONVERSATIONS, MAX_USERS, SlackService},
};

#[derive(Debug, Parser)]
#[command(
    name = "lurkline",
    version,
    about = "Read-only Slack access through an existing browser session"
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
    /// Import and manage secure browser-session credential profiles.
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
    /// Read a message thread.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// Fetch an exact message.
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },
    /// Find workspace users.
    Users {
        #[command(subcommand)]
        command: UsersCommand,
    },
    /// Run the read-only MCP server over stdin/stdout.
    Mcp,
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
    /// Remove one profile from the OS credential store and local registry.
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
        Command::Users {
            command: UsersCommand::Find { query, limit, json },
        } => print_users(service.find_users(&query, limit).await?, json),
        Command::Mcp => mcp::serve_stdio(service).await,
    }
}

fn print_rendered(rendered: RenderedMessage, json: bool) -> Result<()> {
    if json {
        print_json(&rendered)
    } else {
        println!("{}", rendered.text);
        Ok(())
    }
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
    if report.conversations.is_empty() && !report.threads.has_unreads {
        println!("Inbox is clear.");
        return Ok(());
    }
    let shown_conversations = report.conversations.len();
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
    if report.has_more_conversations {
        println!(
            "more-conversations\tshown={}\ttotal={}",
            shown_conversations, report.total_unread_conversations
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
            Cli::try_parse_from(["lurkline", "mcp"]).unwrap().command,
            Command::Mcp
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
                permalink: None,
            };
            assert_eq!(
                format_search_match(&message),
                format!("100.000001\t{id}\t{name}\tU456\thello\\r\\u{{1b}}")
            );
        }
    }
}
