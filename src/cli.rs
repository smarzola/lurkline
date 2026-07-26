use std::fmt::Write as _;

use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::{
    config::Config,
    error::{Error, Result},
    http::SlackHttpClient,
    mcp,
    model::{
        Conversation, ConversationKind, ConversationPage, ConversationSearchReport,
        ConversationSearchTruncationReason, DoctorReport, Message, MessagePage, MessageSearchPage,
        ThreadPage, UnreadReport, UserSearchReport, UserSearchTruncationReason,
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
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
pub enum ChannelCommand {
    /// Read recent channel history.
    Read {
        /// Slack conversation ID or unambiguous exact name.
        channel_id: String,
        /// Maximum messages to return.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ThreadCommand {
    /// Read a thread root and its replies.
    Read {
        /// Slack conversation ID or unambiguous exact name.
        channel_id: String,
        /// Slack timestamp of the thread root.
        thread_ts: String,
        /// Maximum messages to return.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MessageCommand {
    /// Fetch one message by channel ID and timestamp.
    Get {
        /// Slack conversation ID or unambiguous exact name.
        channel_id: String,
        /// Exact Slack message timestamp.
        message_ts: String,
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
        /// Maximum conversations to return.
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
        /// Maximum conversations to return.
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
        /// Restrict to a conversation ID or unambiguous exact name.
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
        /// Maximum matching messages to return.
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
        /// Maximum users to return.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit stable JSON.
        #[arg(long)]
        json: bool,
    },
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let config = Config::from_env()?;
    let client = SlackHttpClient::new(config.clone())?;
    let service = SlackService::new(client, &config);
    match cli.command {
        Command::Doctor { json } => print_doctor(service.doctor().await?, json),
        Command::Unreads { json } => print_unreads(service.unreads().await?, json),
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
                    limit,
                    json,
                },
        } => print_message_page(service.read_channel(&channel_id, limit).await?, json),
        Command::Thread {
            command:
                ThreadCommand::Read {
                    channel_id,
                    thread_ts,
                    limit,
                    json,
                },
        } => print_thread_page(
            service.read_thread(&channel_id, &thread_ts, limit).await?,
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
        Command::Users {
            command: UsersCommand::Find { query, limit, json },
        } => print_users(service.find_users(&query, limit).await?, json),
        Command::Mcp => mcp::serve_stdio(service).await,
    }
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|_| Error::Output)?
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
                "lurkline", "channel", "read", "C123", "--limit", "12", "--json"
            ])
            .unwrap()
            .command,
            Command::Channel {
                command: ChannelCommand::Read {
                    limit: 12,
                    json: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "thread", "read", "C123", "100.000001"])
                .unwrap()
                .command,
            Command::Thread {
                command: ThreadCommand::Read { limit: 100, .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["lurkline", "message", "get", "C123", "100.000001"])
                .unwrap()
                .command,
            Command::Message { .. }
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
                permalink: None,
            };
            assert_eq!(
                format_search_match(&message),
                format!("100.000001\t{id}\t{name}\tU456\thello\\r\\u{{1b}}")
            );
        }
    }
}
