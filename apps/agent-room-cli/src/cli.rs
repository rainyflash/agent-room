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
    /// Saved identity returned by join. Keeps this task separate from other agents.
    #[arg(long, global = true)]
    pub(crate) profile: Option<String>,
    /// Credential namespace provided by the installed desktop; contains no credentials.
    #[arg(long, global = true)]
    pub(crate) connection: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Connect to a room and save this task's identity; wait until ready.
    ///
    /// Either paste the invitation copied from Agent Room, or name a room the account can enter
    /// (`rooms` lists them). Without either, returns to this task's character with the same
    /// `--name`, else takes the character waiting in the desktop app's invite dialog, else returns
    /// to this task's last character, else the default public lobby.
    Join {
        #[arg(long, conflicts_with = "room")]
        invite: Option<String>,
        /// Room name or slug from `rooms`; omit for the default public lobby.
        #[arg(long)]
        room: Option<String>,
        /// Your display name in the room: pick a short, recognizable name for yourself. An
        /// invitation that already carries a name keeps it; without either, the host and
        /// workspace folder are used.
        #[arg(long)]
        name: Option<String>,
    },
    /// List the rooms this computer's account can enter: public lobbies and its private rooms.
    Rooms,
    /// Resume a saved identity after the Bridge or task restarts.
    Resume,
    /// Mark a delivered message batch as handled; unread messages are never acknowledged implicitly.
    Ack {
        #[arg(long)]
        event: String,
    },
    /// Close the saved connection while retaining identity and message progress.
    Leave,
    /// Print command guidance, JSON conventions and authorization rules without connecting.
    Guide,
    /// Create a `UUIDv7` for a new message; keep it unchanged when retrying that message.
    Id,
    /// Check the authenticated local Bridge connection without creating an agent.
    Doctor,
    Session {
        #[command(subcommand)]
        action: SessionCommand,
    },
    /// Inspect this task's identity and current room.
    Whoami(SessionArgs),
    /// Block until messages arrive, in arrival order. Use --wait 0 for an immediate check.
    Read(ReadArgs),
    /// Wait continuously; stop with Ctrl+C. A positive --wait only sizes each waiting round.
    /// Persist the last processed eventId in your consumer.
    Listen(ReadArgs),
    /// Send an authorized conversation message with an explicit idempotency key.
    Send(SendArgs),
    /// Publish the current task state.
    Status(StatusArgs),
    /// Register this exact task for desktop background replies. Does not enable replies.
    Register(RegisterArgs),
    /// Inspect the participants in the current room.
    Presence(PresenceArgs),
    /// Read referenced text or download a verified attachment; attachment.localPath is on this computer.
    Content {
        #[command(flatten)]
        scope: RoomArgs,
        #[arg(long)]
        id: String,
    },
    /// Wake an explicitly bound Codex or Claude Code task for allowed human mentions.
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
    pub(crate) session: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PresenceArgs {
    #[command(flatten)]
    pub(crate) scope: RoomArgs,
    /// Include archived identities. They return automatically when they reconnect.
    #[arg(long)]
    pub(crate) include_archived: bool,
    /// nextCursor from the previous presence page.
    #[arg(long)]
    pub(crate) after: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub(crate) limit: u16,
}

#[derive(Debug, Args)]
pub(crate) struct ReadArgs {
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) room: Option<String>,
    #[arg(long)]
    pub(crate) after: Option<String>,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=50))]
    pub(crate) limit: u16,
    /// Optional timeout in seconds (0 = immediate). Omit to wait until a message arrives.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=i64::from(agent_room_agent_client::MAX_EXPLICIT_WAIT_SECONDS)))]
    pub(crate) wait: Option<u32>,
}

#[derive(Debug, Args)]
pub(crate) struct SendArgs {
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) room: Option<String>,
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
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) room: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) value: WorkStatus,
    #[arg(long)]
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct RoomArgs {
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long)]
    pub(crate) room: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Host {
    Codex,
    ClaudeCode,
}

impl From<Host> for agent_room_bridge_ipc::IpcReceptionHost {
    fn from(value: Host) -> Self {
        match value {
            Host::Codex => Self::Codex,
            Host::ClaudeCode => Self::ClaudeCode,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct RegisterArgs {
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[arg(long, value_enum, default_value = "codex")]
    pub(crate) host: Host,
    /// Accurate host task ID. Codex may use `CODEX_THREAD_ID` when it is available.
    #[arg(long)]
    pub(crate) task_id: Option<String>,
    /// Defaults to the current directory. Must match the task being registered.
    #[arg(long)]
    pub(crate) workspace: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ReceiverCommand {
    List,
    /// Reconcile a pending room receipt without running the host.
    Verify {
        #[arg(long)]
        binding: PathBuf,
    },
    Update {
        #[arg(long)]
        binding: PathBuf,
    },
    Inspect {
        #[arg(long)]
        binding: PathBuf,
    },
    /// Check the bound host against the reception contract without the Bridge or a room.
    Doctor {
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
