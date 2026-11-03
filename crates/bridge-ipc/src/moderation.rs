//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”，IPC 4.6 起）：主人给了
//! 管理授权的房间里，Agent 读规则、照规则禁言、踢出、封禁、解开它封的。Bridge 交给控制面，能不能做由
//! 控制面按授权把关。包在 `WithSession` 里用，房间是会话的 Agent 在的那个。

use agent_room_domain::{
    moderation::ModerationReason,
    room_moderation_grants::{
        MAX_AGENT_MODERATION_NOTE_CHARACTERS, MAX_AGENT_MUTE_MILLIS, MIN_AGENT_MUTE_MILLIS,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    host_sessions::validate_session_id,
    limits,
    tools::{IpcMethodValidationFailure, failure, validate_human_text},
};

/// Agent 要做的：先读规则，再照规则动手。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcModerationAction {
    /// 读主人给的规则、能做哪几样、到什么时候。没有授权时回答里没有。
    Rules,
    Mute,
    Kick,
    Ban,
    /// 只解它自己（照授权）封的。
    Unban,
}

impl IpcModerationAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Mute => "mute",
            Self::Kick => "kick",
            Self::Ban => "ban",
            Self::Unban => "unban",
        }
    }
}

/// 管房间（`ModerateRoom`）。读规则只给 `action`；动手要给对象（发言人的聊天账号，消息里的
/// `from`）和理由（`note`，会写进台账给房主看）；禁言、踢出、封禁还要给原因，禁言可以给时长。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcModerateRoomRequest {
    pub action: IpcModerationAction,
    /// 管哪个房间：会话的 Agent 在的、用目录 ID 说。不给就是开会话时进的那个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// `spam`、`harassment` 这类治理原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 只有禁言收；不给就一直禁着，直到有人解开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
}

impl IpcModerateRoomRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        if let Some(catalog_id) = &self.catalog_id {
            validate_session_id(catalog_id).map_err(|_| failure("bridge.ipc.room_invalid"))?;
        }
        if self.action == IpcModerationAction::Rules {
            let extra = self.target.is_some()
                || self.reason.is_some()
                || self.note.is_some()
                || self.duration_seconds.is_some();
            return if extra {
                Err(failure("bridge.ipc.moderation_invalid"))
            } else {
                Ok(())
            };
        }
        let target = self
            .target
            .as_deref()
            .ok_or(failure("bridge.ipc.moderation_target_invalid"))?;
        if !target.starts_with('@')
            || target.len() > limits::MESSAGE_FROM_BYTES
            || target
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(failure("bridge.ipc.moderation_target_invalid"));
        }
        validate_human_text(
            self.note.as_deref().unwrap_or_default(),
            MAX_AGENT_MODERATION_NOTE_CHARACTERS,
            "bridge.ipc.moderation_note_invalid",
        )?;
        self.validate_reason_and_duration()
    }

    /// 解封不收原因；禁言、踢出、封禁要一个认得的原因。时长只有禁言收，在一分钟到 30 天之间。
    fn validate_reason_and_duration(&self) -> Result<(), IpcMethodValidationFailure> {
        let unban = self.action == IpcModerationAction::Unban;
        match self.reason.as_deref() {
            None if unban => {}
            Some(reason) if !unban && ModerationReason::try_from(reason).is_ok() => {}
            _ => return Err(failure("bridge.ipc.moderation_reason_invalid")),
        }
        match self.duration_seconds {
            None => Ok(()),
            Some(seconds)
                if self.action == IpcModerationAction::Mute
                    && seconds.checked_mul(1_000).is_some_and(|millis| {
                        (MIN_AGENT_MUTE_MILLIS..=MAX_AGENT_MUTE_MILLIS).contains(&millis)
                    }) =>
            {
                Ok(())
            }
            Some(_) => Err(failure("bridge.ipc.moderation_duration_invalid")),
        }
    }
}

/// 主人给的管理授权：照什么规则、能做哪几样（`mute`、`kick`、`ban`、`unban`）、到什么时候。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcModerationGrant {
    pub rules: String,
    pub actions: Vec<String>,
    pub expires_at_unix_ms: i64,
}

/// 台账里记下的这一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcModerationOutcome {
    pub action_id: String,
    pub action: String,
    pub status: String,
    /// 限时禁言什么时候解开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::{IpcModerateRoomRequest, IpcModerationAction};
    use crate::IpcMethod;

    fn mute() -> IpcModerateRoomRequest {
        IpcModerateRoomRequest {
            action: IpcModerationAction::Mute,
            catalog_id: None,
            target: Some("@spammer:matrix.test".to_owned()),
            reason: Some("spam".to_owned()),
            note: Some("Posted the same link five times.".to_owned()),
            duration_seconds: Some(600),
        }
    }

    fn code(request: IpcModerateRoomRequest) -> Option<&'static str> {
        IpcMethod::ModerateRoom(request)
            .validate()
            .err()
            .map(crate::tools::IpcMethodValidationFailure::code)
    }

    #[test]
    fn 读规则只给动作_动手要对象理由和认得的原因() {
        let rules = IpcModerateRoomRequest {
            action: IpcModerationAction::Rules,
            catalog_id: None,
            target: None,
            reason: None,
            note: None,
            duration_seconds: None,
        };
        assert_eq!(code(rules.clone()), None);
        assert_eq!(
            code(IpcModerateRoomRequest {
                target: Some("@someone:matrix.test".to_owned()),
                ..rules
            }),
            Some("bridge.ipc.moderation_invalid")
        );
        assert_eq!(code(mute()), None);
        assert_eq!(
            code(IpcModerateRoomRequest {
                target: Some("spammer".to_owned()),
                ..mute()
            }),
            Some("bridge.ipc.moderation_target_invalid")
        );
        assert_eq!(
            code(IpcModerateRoomRequest {
                note: None,
                ..mute()
            }),
            Some("bridge.ipc.moderation_note_invalid")
        );
        assert_eq!(
            code(IpcModerateRoomRequest {
                reason: Some("rude".to_owned()),
                ..mute()
            }),
            Some("bridge.ipc.moderation_reason_invalid")
        );
    }

    #[test]
    fn 解封不收原因_时长只有禁言收_在一分钟到三十天之间() {
        let unban = IpcModerateRoomRequest {
            action: IpcModerationAction::Unban,
            reason: None,
            duration_seconds: None,
            ..mute()
        };
        assert_eq!(code(unban.clone()), None);
        assert_eq!(
            code(IpcModerateRoomRequest {
                reason: Some("spam".to_owned()),
                ..unban
            }),
            Some("bridge.ipc.moderation_reason_invalid")
        );
        assert_eq!(
            code(IpcModerateRoomRequest {
                action: IpcModerationAction::Ban,
                ..mute()
            }),
            Some("bridge.ipc.moderation_duration_invalid")
        );
        assert_eq!(
            code(IpcModerateRoomRequest {
                duration_seconds: Some(59),
                ..mute()
            }),
            Some("bridge.ipc.moderation_duration_invalid")
        );
        assert_eq!(
            code(IpcModerateRoomRequest {
                duration_seconds: Some(30 * 24 * 60 * 60),
                ..mute()
            }),
            None
        );
    }

    #[test]
    fn 报文是驼峰的_多出来的字段不认() {
        let encoded = serde_json::to_value(IpcMethod::ModerateRoom(mute())).unwrap();
        assert_eq!(
            encoded["moderate_room"],
            serde_json::json!({
                "action": "mute",
                "target": "@spammer:matrix.test",
                "reason": "spam",
                "note": "Posted the same link five times.",
                "durationSeconds": 600
            })
        );
        assert!(
            serde_json::from_value::<IpcModerateRoomRequest>(
                serde_json::json!({"action": "rules", "scope": "all"})
            )
            .is_err()
        );
    }
}
