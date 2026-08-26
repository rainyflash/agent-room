mod cli;
mod commands;
mod error;
mod files;
mod keys;

use clap::Parser as _;

use crate::{
    cli::{Cli, Command},
    error::ToolResult,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("发布工具失败：{error}");
        std::process::exit(1);
    }
}

fn run() -> ToolResult<()> {
    match Cli::parse().command {
        Command::Keygen(args) => commands::keygen(&args),
        Command::Sign(args) => commands::sign(&args),
        Command::Verify(args) => commands::verify(&args),
    }
}
