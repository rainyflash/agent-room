//! 桌面端接入面板“等待接入”的那一份邀请：只放在 Bridge 内存里，同一时间最多一份，
//! 面板不续期就在十分钟后失效。Agent 用它开出会话时才算用掉，所以开会话失败后重试仍能接上。

use std::{sync::Mutex, time::Duration};

use agent_room_bridge_ipc::{IpcOpenHostSessionRequest, IpcPendingInvitation};
use tokio::time::Instant;

pub(crate) const INVITATION_LIFETIME: Duration = Duration::from_mins(10);

pub(crate) struct InvitationSlot {
    lifetime: Duration,
    slot: Mutex<Option<(IpcOpenHostSessionRequest, Instant)>>,
}

impl Default for InvitationSlot {
    fn default() -> Self {
        Self::new(INVITATION_LIFETIME)
    }
}

impl InvitationSlot {
    pub(crate) fn new(lifetime: Duration) -> Self {
        Self {
            lifetime,
            slot: Mutex::new(None),
        }
    }

    /// 挂上或续期；后挂的替换先挂的。
    pub(crate) fn offer(&self, invitation: IpcOpenHostSessionRequest) -> IpcPendingInvitation {
        *self.lock() = Some((invitation.clone(), Instant::now() + self.lifetime));
        IpcPendingInvitation {
            invitation,
            expires_in_ms: millis(self.lifetime),
        }
    }

    /// 只撤回同一会话键的那份，旧面板撤不掉新面板挂的邀请。
    pub(crate) fn withdraw(&self, session_key: &str) {
        let mut slot = self.lock();
        if slot
            .as_ref()
            .is_some_and(|(invitation, _)| invitation.session_key == session_key)
        {
            *slot = None;
        }
    }

    /// 看一眼等待中的邀请；过期的顺手清掉。
    pub(crate) fn read(&self) -> Option<IpcPendingInvitation> {
        let mut slot = self.lock();
        let now = Instant::now();
        match slot.as_ref() {
            Some((invitation, expires)) if *expires > now => Some(IpcPendingInvitation {
                invitation: invitation.clone(),
                expires_in_ms: millis(*expires - now),
            }),
            Some(_) => {
                *slot = None;
                None
            }
            None => None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(IpcOpenHostSessionRequest, Instant)>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use agent_room_bridge_ipc::IpcOpenHostSessionRequest;
    use uuid::Uuid;

    use super::InvitationSlot;

    fn invitation(name: &str) -> IpcOpenHostSessionRequest {
        IpcOpenHostSessionRequest {
            session_key: Uuid::now_v7().to_string(),
            display_name: name.to_owned(),
            room: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn 只留最后挂的一份_看不会用掉_过期或撤回后没有() {
        let slot = InvitationSlot::new(Duration::from_mins(1));
        assert!(slot.read().is_none());
        let first = invitation("Scout");
        slot.offer(first.clone());
        let second = invitation("Pilot");
        assert_eq!(slot.offer(second.clone()).expires_in_ms, 60_000);
        // 后挂的替换先挂的；旧键撤不掉新邀请。
        slot.withdraw(&first.session_key);
        assert_eq!(slot.read().expect("仍在等待").invitation, second);
        // 看一眼不算用掉，开会话失败后重试仍能接上。
        assert_eq!(slot.read().expect("仍在等待").invitation, second);
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(slot.read().expect("未过期").expires_in_ms, 30_000);
        // 续期从续的那一刻重新算。
        slot.offer(second.clone());
        tokio::time::advance(Duration::from_secs(45)).await;
        assert!(slot.read().is_some());
        tokio::time::advance(Duration::from_secs(16)).await;
        assert!(slot.read().is_none());
        slot.offer(second.clone());
        slot.withdraw(&second.session_key);
        assert!(slot.read().is_none());
    }
}
