//! 收到的加密消息的发送设备信任状态可能已经过时。
//!
//! SDK 在解密那一刻按本机缓存判定发送设备是否由其主人签名。对方如果刚建立加密身份，
//! 本机缓存可能还停在它签名之前，消息就会被标成「设备未签名」或「设备未知」而被隔离。
//! 这里先向服务端刷新这些发送者的设备与身份，再按当前状态重新判定。

use std::collections::BTreeSet;

use matrix_sdk::{
    Client,
    deserialized_responses::{
        DeviceLinkProblem, TimelineEvent, VerificationLevel, VerificationState,
    },
    ruma::{OwnedEventId, OwnedRoomId, OwnedUserId, RoomId},
};

use crate::{
    mapping::sender_device_trusted,
    owner::{AgentOwner, accept_owner_identity},
};

/// 刷新后确认由主人签名的事件。映射时视为可信。
#[derive(Debug, Default)]
pub(crate) struct SenderTrustUpgrades(BTreeSet<OwnedEventId>);

impl SenderTrustUpgrades {
    pub(crate) fn insert(&mut self, event_id: OwnedEventId) {
        self.0.insert(event_id);
    }

    pub(crate) fn contains(&self, event: &TimelineEvent) -> bool {
        event
            .event_id()
            .is_some_and(|event_id| self.0.contains(&event_id))
    }
}

struct Candidate {
    room_id: OwnedRoomId,
    event_id: OwnedEventId,
    session_id: String,
    sender: OwnedUserId,
}

/// 对可能因缓存过时而判为不可信的事件，刷新发送者并重新判定。
///
/// 只处理「设备未签名」和「设备未知」：发送者不符、核对后又换了身份等是真实问题，不做刷新。
/// 例外是主人核对过之后换了身份（设备自动签名重建过，见 `owner.rs`）：撤销旧核对后重判。
/// 刷新失败时维持原判定，照常隔离。
pub(crate) async fn refresh_stale_sender_trust<'a>(
    client: &Client,
    owner: &AgentOwner,
    rooms: impl IntoIterator<Item = (&'a RoomId, &'a [TimelineEvent])>,
) -> SenderTrustUpgrades {
    let mut candidates = Vec::new();
    for (room_id, events) in rooms {
        for event in events {
            let Some(info) = event.encryption_info() else {
                continue;
            };
            let rejudge = possibly_stale(&info.verification_state)
                || (owner.is(&info.sender) && owner_changed_identity(&info.verification_state));
            if !rejudge {
                continue;
            }
            let (Some(event_id), Some(session_id)) = (event.event_id(), info.session_id()) else {
                continue;
            };
            candidates.push(Candidate {
                room_id: room_id.to_owned(),
                event_id,
                session_id: session_id.to_owned(),
                sender: info.sender.clone(),
            });
        }
    }
    let mut upgrades = SenderTrustUpgrades::default();
    if candidates.is_empty() {
        return upgrades;
    }
    let senders = candidates
        .iter()
        .map(|candidate| candidate.sender.clone())
        .collect::<BTreeSet<_>>();
    for sender in &senders {
        // 刷新失败时下面按缓存重判，结果与原判定相同，事件照常隔离。
        if let Ok(Some(identity)) = client.encryption().request_user_identity(sender).await
            && owner.is(sender)
        {
            accept_owner_identity(&identity).await;
        }
    }
    for candidate in candidates {
        let Some(room) = client.get_room(&candidate.room_id) else {
            continue;
        };
        if room
            .get_encryption_info(&candidate.session_id, &candidate.sender)
            .await
            .is_some_and(|info| sender_device_trusted(&info.verification_state))
        {
            upgrades.0.insert(candidate.event_id);
        }
    }
    upgrades
}

const fn owner_changed_identity(state: &VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Unverified(VerificationLevel::VerificationViolation)
    )
}

const fn possibly_stale(state: &VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Unverified(
            VerificationLevel::UnsignedDevice
                | VerificationLevel::None(DeviceLinkProblem::MissingDevice)
        )
    )
}

#[cfg(test)]
mod tests {
    use matrix_sdk::deserialized_responses::{
        DeviceLinkProblem, VerificationLevel, VerificationState,
    };

    use super::possibly_stale;

    #[test]
    fn 只有未签名或未知的发送设备才值得刷新重判() {
        for state in [
            VerificationLevel::UnsignedDevice,
            VerificationLevel::None(DeviceLinkProblem::MissingDevice),
        ] {
            assert!(possibly_stale(&VerificationState::Unverified(state)));
        }
        for state in [
            VerificationState::Verified,
            VerificationState::Unverified(VerificationLevel::UnverifiedIdentity),
            VerificationState::Unverified(VerificationLevel::VerificationViolation),
            VerificationState::Unverified(VerificationLevel::MismatchedSender),
            VerificationState::Unverified(VerificationLevel::None(
                DeviceLinkProblem::InsecureSource,
            )),
        ] {
            assert!(!possibly_stale(&state));
        }
    }
}
