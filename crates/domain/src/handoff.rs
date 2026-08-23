use crate::{
    DomainError, DomainResult,
    ids::{AgentInstanceId, ContentId, HandoffId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffStatus {
    Requested,
    Accepted,
    Rejected,
    Consumed,
    Expired,
}

impl HandoffStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Consumed => "consumed",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHandoff {
    id: HandoffId,
    content_id: ContentId,
    target_instance_id: AgentInstanceId,
    status: HandoffStatus,
}

impl ContextHandoff {
    pub const fn request(
        id: HandoffId,
        content_id: ContentId,
        target_instance_id: AgentInstanceId,
    ) -> Self {
        Self {
            id,
            content_id,
            target_instance_id,
            status: HandoffStatus::Requested,
        }
    }

    pub const fn id(&self) -> HandoffId {
        self.id
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    pub const fn target_instance_id(&self) -> AgentInstanceId {
        self.target_instance_id
    }

    pub const fn status(&self) -> HandoffStatus {
        self.status
    }

    /// 接受上下文交付请求，重复接受保持幂等。
    ///
    /// # Errors
    ///
    /// 已拒绝、已消费或已过期请求不能接受。
    pub fn accept(&mut self) -> DomainResult<()> {
        match self.status {
            HandoffStatus::Requested | HandoffStatus::Accepted => {
                self.status = HandoffStatus::Accepted;
                Ok(())
            }
            status => Err(DomainError::InvalidTransition {
                entity: "context_handoff",
                from: status.label(),
                to: "accepted",
            }),
        }
    }

    /// 拒绝上下文交付请求，重复拒绝保持幂等。
    ///
    /// # Errors
    ///
    /// 已接受、已消费或已过期请求不能拒绝。
    pub fn reject(&mut self) -> DomainResult<()> {
        match self.status {
            HandoffStatus::Requested | HandoffStatus::Rejected => {
                self.status = HandoffStatus::Rejected;
                Ok(())
            }
            status => Err(DomainError::InvalidTransition {
                entity: "context_handoff",
                from: status.label(),
                to: "rejected",
            }),
        }
    }

    /// 标记已接受的上下文包完成消费，重复回执保持幂等。
    ///
    /// # Errors
    ///
    /// 未接受、已拒绝或已过期请求不能消费。
    pub fn consume(&mut self) -> DomainResult<()> {
        match self.status {
            HandoffStatus::Accepted | HandoffStatus::Consumed => {
                self.status = HandoffStatus::Consumed;
                Ok(())
            }
            status => Err(DomainError::InvalidTransition {
                entity: "context_handoff",
                from: status.label(),
                to: "consumed",
            }),
        }
    }

    /// 使等待或已接受的上下文交付过期。
    ///
    /// # Errors
    ///
    /// 已拒绝或已消费交付不能改写为过期。
    pub fn expire(&mut self) -> DomainResult<()> {
        match self.status {
            HandoffStatus::Requested | HandoffStatus::Accepted | HandoffStatus::Expired => {
                self.status = HandoffStatus::Expired;
                Ok(())
            }
            status => Err(DomainError::InvalidTransition {
                entity: "context_handoff",
                from: status.label(),
                to: "expired",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{ContextHandoff, HandoffStatus};
    use crate::ids::{AgentInstanceId, ContentId, HandoffId};

    #[test]
    fn 未接受的上下文不能被消费() {
        let mut handoff = ContextHandoff::request(
            HandoffId::from_uuid(Uuid::from_u128(1)),
            ContentId::from_uuid(Uuid::from_u128(2)),
            AgentInstanceId::from_uuid(Uuid::from_u128(3)),
        );

        assert!(handoff.consume().is_err());
        handoff.accept().expect("用户确认后允许接受");
        handoff.consume().expect("接受后允许消费");
        handoff.consume().expect("消费回执必须幂等");
        assert_eq!(handoff.status(), HandoffStatus::Consumed);
    }
}
