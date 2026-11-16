//! 须知和置顶共用的次数上限（`specs/redesign/brief-and-pins.md`“接口”）：两样都是应用服务替人写的房间
//! 状态，每写一次就是一条新的状态事件，聊天服务器不删，应用服务又不受它的限速，所以每个人一小时里
//! 两样合起来最多写 [`ROOM_STATE_WRITES_PER_HOUR`] 次。控制面里只建一份，交给两个用例。

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use crate::hourly_limit::{HourlyLimit, HourlyLimitReached};

/// 每个人一小时里改须知、置顶合起来最多几次。日常改几下够用，又挡住拿它刷状态事件。
pub const ROOM_STATE_WRITES_PER_HOUR: usize = 30;

pub struct RoomStateWriteLimit(HourlyLimit);

impl RoomStateWriteLimit {
    #[must_use]
    pub fn new() -> Self {
        Self(HourlyLimit::new(ROOM_STATE_WRITES_PER_HOUR))
    }

    /// 记一次；这一小时已经满了时不记，告诉什么时候能再来。
    pub(crate) fn take(
        &self,
        principal: PrincipalId,
        now: UtcMillis,
    ) -> Result<(), HourlyLimitReached> {
        self.0.take(principal, now)
    }
}

impl Default for RoomStateWriteLimit {
    fn default() -> Self {
        Self::new()
    }
}
