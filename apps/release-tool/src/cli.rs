use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "agent-room-release", about = "Agent Room 离线发布签名工具")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Keygen(KeygenArgs),
    Sign(SignArgs),
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct KeygenArgs {
    #[arg(long)]
    pub private_key: PathBuf,
    #[arg(long)]
    pub public_key: PathBuf,
}

#[derive(Debug, Args)]
pub struct SignArgs {
    #[arg(long)]
    pub private_key: PathBuf,
    #[arg(long)]
    pub manifest: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ChannelArg {
    Stable,
    Testing,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[arg(long)]
    pub public_key: PathBuf,
    #[arg(long)]
    pub manifest: PathBuf,
    #[arg(long, value_enum)]
    pub channel: ChannelArg,
    #[arg(long)]
    pub installed_version: String,
    #[arg(long)]
    pub highest_sequence: u64,
    #[arg(long)]
    pub now_unix_seconds: Option<u64>,
}
