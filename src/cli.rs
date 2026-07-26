use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::{
    config::Config,
    error::{Error, Result},
    http::SlackHttpClient,
    model::{ConversationKind, DoctorReport, UnreadReport},
    service::SlackService,
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
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let config = Config::from_env()?;
    let client = SlackHttpClient::new(config.clone())?;
    let service = SlackService::new(client, &config);
    match cli.command {
        Command::Doctor { json } => print_doctor(service.doctor().await?, json),
        Command::Unreads { json } => print_unreads(service.unreads().await?, json),
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
        let kind = match conversation.kind {
            ConversationKind::Channel => "channel",
            ConversationKind::DirectMessage => "dm",
            ConversationKind::GroupDirectMessage => "group-dm",
        };
        println!(
            "{kind}\t{}\tmentions={}\tlast_read={}\tlatest={}",
            conversation.id,
            conversation.mention_count,
            conversation.last_read.as_deref().unwrap_or("-"),
            conversation.latest.as_deref().unwrap_or("-")
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
}
