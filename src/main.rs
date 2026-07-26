use clap::Parser;
use lurkline::{Cli, run_cli};

#[tokio::main]
async fn main() {
    if let Err(error) = run_cli(Cli::parse()).await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
