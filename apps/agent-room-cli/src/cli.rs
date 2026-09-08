use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "agent-room",
    version,
    about = "Agent Room tools. Command results are JSON; listen emits one JSON result per batch."
)]
pub(crate) struct Cli {
    #[arg(long, global = true)]
    pub(crate) data_root: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Check the authenticated local Bridge connection without creating an agent.
    Doctor,
    Session {
        #[command(subcommand)]
        action: SessionCommand,
    },
    /// Inspect this task's identity and current room.
    Whoami(SessionArgs),
    /// Read messages in arrival order. Continue using the last returned eventId.
    Read(ReadArgs),
    /// Wait continuously; stop with Ctrl+C. Persist the last processed eventId in your consumer.
    Listen(ReadArgs),
    /// Send an authorized conversation message with an explicit idempotency key.
    Send(SendArgs),
    /// Publish the current task state.
    Status(StatusArgs),
    /// Wake an explicitly bound Codex task for allowed human mentions.
    Receive {
        #[arg(long)]
        binding: PathBuf,
    },
    /// Inspect or resolve an uncertain delivery after reviewing the host task.
    Receiver {
        #[command(subcommand)]
        action: ReceiverCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    Open {
        #[arg(long)]
        name: String,
        #[arg(long)]
        key: Option<String>,
    },
    Close(SessionArgs),
}

#[derive(Debug, Args)]
pub(crate) struct SessionArgs {
    #[arg(long)]
    pub(crate) session: String,
}

#[derive(Debug, Args)]
pub(crate) struct ReadArgs {
    #[arg(long)]
    pub(crate) session: String,
    #[arg(long)]
    pub(crate) room: Option<String>,
    #[arg(long)]
    pub(crate) after: Option<String>,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=50))]
    pub(crate) limit: u16,
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u8).range(0..=25))]
    pub(crate) wait: u8,
}

#[derive(Debug, Args)]
pub(crate) struct SendArgs {
    #[arg(long)]
    pub(crate) session: String,
    #[arg(long)]
    pub(crate) room: String,
    #[arg(long, required_unless_present = "stdin", conflicts_with = "stdin")]
    pub(crate) text: Option<String>,
    #[arg(long)]
    pub(crate) stdin: bool,
    #[arg(long)]
    pub(crate) submission_id: String,
    #[arg(long)]
    pub(crate) reply_to: Option<String>,
    #[arg(long)]
    pub(crate) mention: Vec<String>,
    #[arg(
        long,
        required_unless_present = "automation_grant",
        conflicts_with = "automation_grant"
    )]
    pub(crate) authorized: bool,
    #[arg(long)]
    pub(crate) automation_grant: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum WorkStatus {
    Offline,
    Idle,
    Working,
    WaitingInput,
    Blocked,
    Completed,
}

#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    #[arg(long)]
    pub(crate) session: String,
    #[arg(long)]
    pub(crate) room: String,
    #[arg(long, value_enum)]
    pub(crate) value: WorkStatus,
    #[arg(long)]
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ReceiverCommand {
    List,
    Update {
        #[arg(long)]
        binding: PathBuf,
    },
    Inspect {
        #[arg(long)]
        binding: PathBuf,
    },
    Resolve {
        #[arg(long)]
        binding: PathBuf,
        #[arg(long)]
        event: String,
        #[arg(long, value_enum)]
        action: Resolution,
    },
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Resolution {
    Retry,
    Skip,
}
