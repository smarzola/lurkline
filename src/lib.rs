mod auth;
pub mod cli;
mod config;
mod error;
mod http;
mod mcp;
mod model;
mod service;

pub use cli::{Cli, run_cli};
