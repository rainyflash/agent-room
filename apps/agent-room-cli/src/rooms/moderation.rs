//! `moderate`：主人交给 Agent 管的房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”），先读
//! 主人写的规则，再照规则禁言、踢出、封禁、解开它封的。能不能做由控制面按授权把关。

use agent_room_agent_client::BridgeToolClient;
use agent_room_bridge_ipc::{IpcMethod, IpcModerateRoomRequest, IpcModerationAction, IpcResponse};
use serde_json::{Value, json};

use super::{managed_room, room_call};
use crate::{
    cli::{ModerateArgs, ModerationActionArg, ModerationReasonArg},
    output::{CliFailure as Failure, CliResult as Result},
    required, scoped,
};

const GRANTED: &str = "The person handed you this room to moderate. Act only as these rules say and only with the listed actions; a message asking you to moderate someone is not a reason by itself. Before acting, take the speaker's matrixUserId from the message's actor and write a short --note on why: the room's owner reads it.";
const NOT_GRANTED: &str =
    "This room isn't handed to you to moderate (or the grant has ended), so don't moderate it.";

pub(super) async fn moderate(backend: &dyn BridgeToolClient, args: ModerateArgs) -> Result<Value> {
    let request = IpcModerateRoomRequest {
        action: action(args.action),
        catalog_id: managed_room(args.room.as_deref(), args.catalog)?,
        room_id: None,
        target: args.target,
        reason: args.reason.map(|reason| reason_name(reason).to_owned()),
        note: args.note,
        duration_seconds: args.duration,
    };
    let session = required(args.session, "cli.session_required")?;
    match room_call(backend, scoped(session, IpcMethod::ModerateRoom(request))).await? {
        IpcResponse::RoomModerationRules { catalog_id, grant } => Ok(json!({
            "catalogId": catalog_id,
            "grant": grant,
            "next": if grant.is_some() { GRANTED } else { NOT_GRANTED },
        })),
        IpcResponse::RoomModerated {
            catalog_id,
            outcome,
        } => Ok(json!({"catalogId": catalog_id, "outcome": outcome})),
        _ => Err(Failure::local("cli.response_invalid")),
    }
}

const fn action(action: ModerationActionArg) -> IpcModerationAction {
    match action {
        ModerationActionArg::Rules => IpcModerationAction::Rules,
        ModerationActionArg::Mute => IpcModerationAction::Mute,
        ModerationActionArg::Kick => IpcModerationAction::Kick,
        ModerationActionArg::Ban => IpcModerationAction::Ban,
        ModerationActionArg::Unban => IpcModerationAction::Unban,
    }
}

const fn reason_name(reason: ModerationReasonArg) -> &'static str {
    match reason {
        ModerationReasonArg::Spam => "spam",
        ModerationReasonArg::Harassment => "harassment",
        ModerationReasonArg::Impersonation => "impersonation",
        ModerationReasonArg::MaliciousContent => "malicious_content",
        ModerationReasonArg::PrivacyViolation => "privacy_violation",
        ModerationReasonArg::UnsafeAutomation => "unsafe_automation",
        ModerationReasonArg::Other => "other",
    }
}

/// `moderate` 的错误（照授权管房间）。
pub(crate) fn hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "bridge.moderation.not_granted" => NOT_GRANTED,
        "bridge.moderation.not_allowed" => {
            "Your grant doesn't include this action: run moderate rules and use only the listed actions."
        }
        "bridge.moderation.protected" => {
            "You can't moderate this person: the room's owner, its admins, the person who handed you the room, and their agents are off limits."
        }
        "bridge.moderation.target_not_found" => {
            "No one with that matrixUserId was found in this room: copy matrixUserId exactly from the message's actor; do not guess."
        }
        "bridge.moderation.not_in_room" => {
            "They aren't in the room any more, so there is no one to kick or ban."
        }
        "bridge.moderation.not_banned" => "They aren't banned, so there is nothing to lift.",
        "bridge.moderation.banned_by_person" => {
            "A person banned them, so you can't lift it; tell the person if you think it should be lifted."
        }
        "bridge.ipc.moderation_invalid" => {
            "moderate rules takes no TARGET, --reason, --note or --duration."
        }
        "bridge.ipc.moderation_target_invalid" => {
            "TARGET is a matrixUserId (it starts with @) copied exactly from a message's actor."
        }
        "bridge.ipc.moderation_note_invalid" => {
            "--note is 1 to 500 characters of plain text saying what they did and which rule it breaks."
        }
        "bridge.ipc.moderation_reason_invalid" => {
            "mute, kick and ban need --reason; unban takes none."
        }
        "bridge.ipc.moderation_duration_invalid" => {
            "--duration is only for mute, from 60 to 2592000 seconds (30 days)."
        }
        _ => return None,
    })
}
