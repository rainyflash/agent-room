use std::collections::BTreeMap;

use crate::{
    DomainError, DomainResult,
    federation::{
        EventCompatibility, FederationDisposition, FederationPolicySet, FederationReputation,
        FederationServerName, ReputationTier,
    },
    ids::FederationRuleId,
    time::{DurationMillis, UtcMillis},
};

const MAX_EVENT_ID_LENGTH: usize = 1_024;
const MAX_EVENT_TYPE_LENGTH: usize = 255;
const MAX_STATE_KEY_LENGTH: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationIngressEvent {
    event_id: String,
    peer: FederationServerName,
    room_id: String,
    sender_user_id: String,
    event_type: String,
    received_at: UtcMillis,
    state: Option<FederationStateRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FederationStateRevision {
    state_key: String,
    revision: u64,
}

impl FederationIngressEvent {
    /// 创建经过大小和结构校验的联邦入口事件描述。
    ///
    /// # Errors
    ///
    /// 任一 Matrix 标识、事件类型或状态字段无效时返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: impl Into<String>,
        peer: impl Into<String>,
        room_id: impl Into<String>,
        sender_user_id: impl Into<String>,
        event_type: impl Into<String>,
        received_at: UtcMillis,
        state_key: Option<String>,
        state_revision: Option<u64>,
    ) -> DomainResult<Self> {
        let event_id = event_id.into();
        let room_id = room_id.into();
        let sender_user_id = sender_user_id.into();
        let event_type = event_type.into();
        validate_matrix_id("federation_event_id", &event_id, '$', MAX_EVENT_ID_LENGTH)?;
        validate_matrix_id("federation_room_id", &room_id, '!', MAX_EVENT_ID_LENGTH)?;
        validate_matrix_id(
            "federation_sender_user_id",
            &sender_user_id,
            '@',
            MAX_EVENT_ID_LENGTH,
        )?;
        validate_text("federation_event_type", &event_type, MAX_EVENT_TYPE_LENGTH)?;
        let state = match (state_key, state_revision) {
            (Some(state_key), Some(revision)) => {
                validate_text("federation_state_key", &state_key, MAX_STATE_KEY_LENGTH)?;
                Some(FederationStateRevision {
                    state_key,
                    revision,
                })
            }
            (None, None) => None,
            _ => {
                return Err(validation(
                    "federation_state_revision",
                    "状态键与版本必须同时出现",
                ));
            }
        };
        Ok(Self {
            event_id,
            peer: FederationServerName::new(peer)?,
            room_id,
            sender_user_id,
            event_type,
            received_at,
            state,
        })
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn event_type(&self) -> &str {
        &self.event_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationRateScope {
    Peer,
    Room,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationReadOnlyReason {
    LegacyNamespace,
    UnknownEventType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationQuarantineReason {
    GovernanceRule,
    HostileReputation,
    StateConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationIngressRejection {
    GovernanceBlock,
    Replay,
    RateLimited(FederationRateScope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederationIngressOutcome {
    AcceptReadWrite {
        matching_rule_ids: Vec<FederationRuleId>,
    },
    AcceptReadOnly {
        reason: FederationReadOnlyReason,
        matching_rule_ids: Vec<FederationRuleId>,
    },
    Quarantine {
        reason: FederationQuarantineReason,
        matching_rule_ids: Vec<FederationRuleId>,
    },
    Reject {
        reason: FederationIngressRejection,
        matching_rule_ids: Vec<FederationRuleId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FederationIngressLimits {
    peer_per_window: u32,
    room_per_window: u32,
    user_per_window: u32,
    window: DurationMillis,
    replay_ttl: DurationMillis,
}

impl FederationIngressLimits {
    /// 创建三层联邦速率预算和重放记忆时限。
    ///
    /// # Errors
    ///
    /// 任一预算为零或层级预算倒置时返回错误。
    pub fn new(
        peer_per_window: u32,
        room_per_window: u32,
        user_per_window: u32,
        window: DurationMillis,
        replay_ttl: DurationMillis,
    ) -> DomainResult<Self> {
        if peer_per_window == 0 || room_per_window == 0 || user_per_window == 0 {
            return Err(validation("federation_ingress_limit", "必须大于零"));
        }
        if user_per_window > room_per_window || room_per_window > peer_per_window {
            return Err(validation(
                "federation_ingress_limit",
                "必须满足用户预算不高于房间预算且房间预算不高于对端预算",
            ));
        }
        Ok(Self {
            peer_per_window,
            room_per_window,
            user_per_window,
            window,
            replay_ttl,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RateKey {
    Peer(String),
    Room(String, String),
    User(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowCounter {
    started_at: UtcMillis,
    count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StateSlot {
    room_id: String,
    event_type: String,
    state_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptedState {
    revision: u64,
    event_id: String,
}

#[derive(Debug)]
pub struct FederationIngressGuard {
    limits: FederationIngressLimits,
    counters: BTreeMap<RateKey, WindowCounter>,
    replay_expirations: BTreeMap<String, UtcMillis>,
    accepted_state: BTreeMap<StateSlot, AcceptedState>,
}

impl FederationIngressGuard {
    pub const fn new(limits: FederationIngressLimits) -> Self {
        Self {
            limits,
            counters: BTreeMap::new(),
            replay_expirations: BTreeMap::new(),
            accepted_state: BTreeMap::new(),
        }
    }

    /// 依次执行治理、信誉、重放、三级速率、状态冲突和协议兼容决策。
    ///
    /// # Errors
    ///
    /// 时间计算溢出时返回领域错误；调用方必须失败关闭，不能跳过检查。
    pub fn inspect(
        &mut self,
        policies: &FederationPolicySet,
        reputation: FederationReputation,
        event: &FederationIngressEvent,
    ) -> DomainResult<FederationIngressOutcome> {
        self.replay_expirations
            .retain(|_, expiration| *expiration > event.received_at);
        let governance = policies.evaluate(
            &event.peer,
            &event.room_id,
            &event.sender_user_id,
            event.received_at,
        );
        if governance.disposition == FederationDisposition::Block {
            return Ok(FederationIngressOutcome::Reject {
                reason: FederationIngressRejection::GovernanceBlock,
                matching_rule_ids: governance.matching_rule_ids,
            });
        }
        if governance.disposition == FederationDisposition::Quarantine {
            self.remember_replay(event)?;
            return Ok(FederationIngressOutcome::Quarantine {
                reason: FederationQuarantineReason::GovernanceRule,
                matching_rule_ids: governance.matching_rule_ids,
            });
        }
        if reputation.tier() == ReputationTier::Hostile {
            self.remember_replay(event)?;
            return Ok(FederationIngressOutcome::Quarantine {
                reason: FederationQuarantineReason::HostileReputation,
                matching_rule_ids: governance.matching_rule_ids,
            });
        }
        if self.replay_expirations.contains_key(&event.event_id) {
            return Ok(FederationIngressOutcome::Reject {
                reason: FederationIngressRejection::Replay,
                matching_rule_ids: governance.matching_rule_ids,
            });
        }

        let divisor = rate_divisor(governance.disposition, reputation.tier());
        let checks = [
            (
                RateKey::Peer(event.peer.as_str().to_owned()),
                self.limits.peer_per_window,
                FederationRateScope::Peer,
            ),
            (
                RateKey::Room(event.peer.as_str().to_owned(), event.room_id.clone()),
                self.limits.room_per_window,
                FederationRateScope::Room,
            ),
            (
                RateKey::User(event.peer.as_str().to_owned(), event.sender_user_id.clone()),
                self.limits.user_per_window,
                FederationRateScope::User,
            ),
        ];
        for (key, base_limit, scope) in checks {
            let limit = (base_limit / divisor).max(1);
            if !consume_window(
                &mut self.counters,
                key,
                event.received_at,
                self.limits.window,
                limit,
            )? {
                return Ok(FederationIngressOutcome::Reject {
                    reason: FederationIngressRejection::RateLimited(scope),
                    matching_rule_ids: governance.matching_rule_ids,
                });
            }
        }

        if self.has_state_conflict(event) {
            self.remember_replay(event)?;
            return Ok(FederationIngressOutcome::Quarantine {
                reason: FederationQuarantineReason::StateConflict,
                matching_rule_ids: governance.matching_rule_ids,
            });
        }
        self.remember_replay(event)?;
        self.remember_state(event);

        Ok(match EventCompatibility::classify(&event.event_type) {
            EventCompatibility::CurrentWritable => FederationIngressOutcome::AcceptReadWrite {
                matching_rule_ids: governance.matching_rule_ids,
            },
            EventCompatibility::LegacyReadOnly => FederationIngressOutcome::AcceptReadOnly {
                reason: FederationReadOnlyReason::LegacyNamespace,
                matching_rule_ids: governance.matching_rule_ids,
            },
            EventCompatibility::UnknownReadOnly => FederationIngressOutcome::AcceptReadOnly {
                reason: FederationReadOnlyReason::UnknownEventType,
                matching_rule_ids: governance.matching_rule_ids,
            },
        })
    }

    fn remember_replay(&mut self, event: &FederationIngressEvent) -> DomainResult<()> {
        self.replay_expirations.insert(
            event.event_id.clone(),
            event.received_at.checked_add(self.limits.replay_ttl)?,
        );
        Ok(())
    }

    fn has_state_conflict(&self, event: &FederationIngressEvent) -> bool {
        let Some(state) = event.state.as_ref() else {
            return false;
        };
        let key = state_key(event, state);
        self.accepted_state
            .get(&key)
            .is_some_and(|accepted| state.revision <= accepted.revision)
    }

    fn remember_state(&mut self, event: &FederationIngressEvent) {
        let Some(state) = event.state.as_ref() else {
            return;
        };
        self.accepted_state.insert(
            state_key(event, state),
            AcceptedState {
                revision: state.revision,
                event_id: event.event_id.clone(),
            },
        );
    }
}

fn state_key(event: &FederationIngressEvent, state: &FederationStateRevision) -> StateSlot {
    StateSlot {
        room_id: event.room_id.clone(),
        event_type: event.event_type.clone(),
        state_key: state.state_key.clone(),
    }
}

fn rate_divisor(disposition: FederationDisposition, reputation: ReputationTier) -> u32 {
    let policy_divisor = match disposition {
        FederationDisposition::Throttle => 4,
        FederationDisposition::Allow
        | FederationDisposition::Quarantine
        | FederationDisposition::Block => 1,
    };
    let reputation_divisor = match reputation {
        ReputationTier::Degraded => 2,
        ReputationTier::Trusted | ReputationTier::Neutral | ReputationTier::Hostile => 1,
    };
    policy_divisor.max(reputation_divisor)
}

fn consume_window(
    counters: &mut BTreeMap<RateKey, WindowCounter>,
    key: RateKey,
    now: UtcMillis,
    window: DurationMillis,
    limit: u32,
) -> DomainResult<bool> {
    let elapsed = now.value().saturating_sub(
        counters
            .get(&key)
            .map_or(now.value(), |counter| counter.started_at.value()),
    );
    let window_millis = i64::try_from(window.value()).map_err(|_| DomainError::TimeOverflow)?;
    let counter = counters.entry(key).or_insert(WindowCounter {
        started_at: now,
        count: 0,
    });
    if elapsed >= window_millis {
        *counter = WindowCounter {
            started_at: now,
            count: 0,
        };
    }
    if counter.count >= limit {
        return Ok(false);
    }
    counter.count = counter.count.saturating_add(1);
    Ok(true)
}

fn validate_matrix_id(
    field: &'static str,
    value: &str,
    prefix: char,
    maximum: usize,
) -> DomainResult<()> {
    validate_text(field, value, maximum)?;
    if !value.starts_with(prefix) || !value.contains(':') {
        return Err(validation(field, "不是完整 Matrix 标识"));
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str, maximum: usize) -> DomainResult<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(validation(field, "为空、包含控制字符或超过长度上限"));
    }
    Ok(())
}

const fn validation(field: &'static str, reason: &'static str) -> DomainError {
    DomainError::Validation { field, reason }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        federation::{
            ACTIVE_EVENT_NAMESPACE, FederationDisposition, FederationIngressEvent,
            FederationIngressGuard, FederationIngressLimits, FederationIngressOutcome,
            FederationIngressRejection, FederationPolicySet, FederationQuarantineReason,
            FederationRateScope, FederationReadOnlyReason, FederationReputation, FederationRule,
            FederationScope, ReputationSignal,
        },
        ids::{FederationRuleId, PrincipalId},
        time::{DurationMillis, UtcMillis},
    };

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("时间有效")
    }

    fn limits() -> FederationIngressLimits {
        FederationIngressLimits::new(
            12,
            8,
            4,
            DurationMillis::new(60_000).expect("窗口有效"),
            DurationMillis::new(300_000).expect("重放时限有效"),
        )
        .expect("入口预算有效")
    }

    fn event(
        sequence: u16,
        user: &str,
        event_type: &str,
        received_at: i64,
    ) -> FederationIngressEvent {
        FederationIngressEvent::new(
            format!("${sequence}:peer.example"),
            "peer.example",
            "!room:local.example",
            user,
            event_type,
            time(received_at),
            None,
            None,
        )
        .expect("入口事件有效")
    }

    fn inspect(
        guard: &mut FederationIngressGuard,
        event: &FederationIngressEvent,
    ) -> FederationIngressOutcome {
        guard
            .inspect(
                &FederationPolicySet::default(),
                FederationReputation::default(),
                event,
            )
            .expect("入口检查成功")
    }

    #[test]
    fn 重放事件被拒绝且未知事件只读展示() {
        let mut guard = FederationIngressGuard::new(limits());
        let unknown = event(1, "@agent:peer.example", "com.example.future.v9", 100);
        assert_eq!(
            inspect(&mut guard, &unknown),
            FederationIngressOutcome::AcceptReadOnly {
                reason: FederationReadOnlyReason::UnknownEventType,
                matching_rule_ids: vec![]
            }
        );
        assert_eq!(
            inspect(&mut guard, &unknown),
            FederationIngressOutcome::Reject {
                reason: FederationIngressRejection::Replay,
                matching_rule_ids: vec![]
            }
        );
    }

    #[test]
    fn 单用户洪泛只耗尽用户预算而不封死其他用户() {
        let mut guard = FederationIngressGuard::new(limits());
        let event_type = format!("{ACTIVE_EVENT_NAMESPACE}.message.preview.v1");
        for sequence in 0..4 {
            assert!(matches!(
                inspect(
                    &mut guard,
                    &event(sequence, "@noisy:peer.example", &event_type, 100)
                ),
                FederationIngressOutcome::AcceptReadWrite { .. }
            ));
        }
        assert_eq!(
            inspect(
                &mut guard,
                &event(5, "@noisy:peer.example", &event_type, 100)
            ),
            FederationIngressOutcome::Reject {
                reason: FederationIngressRejection::RateLimited(FederationRateScope::User),
                matching_rule_ids: vec![]
            }
        );
        assert!(matches!(
            inspect(
                &mut guard,
                &event(6, "@quiet:peer.example", &event_type, 100)
            ),
            FederationIngressOutcome::AcceptReadWrite { .. }
        ));
    }

    #[test]
    fn 状态倒退被隔离而不覆盖权威状态() {
        let mut guard = FederationIngressGuard::new(limits());
        let event_type = format!("{ACTIVE_EVENT_NAMESPACE}.agent.status.v1");
        let first = FederationIngressEvent::new(
            "$1:peer.example",
            "peer.example",
            "!room:local.example",
            "@agent:peer.example",
            event_type.clone(),
            time(100),
            Some("instance-1".to_owned()),
            Some(8),
        )
        .expect("状态事件有效");
        let stale = FederationIngressEvent::new(
            "$2:peer.example",
            "peer.example",
            "!room:local.example",
            "@agent:peer.example",
            event_type,
            time(101),
            Some("instance-1".to_owned()),
            Some(7),
        )
        .expect("状态事件有效");
        assert!(matches!(
            inspect(&mut guard, &first),
            FederationIngressOutcome::AcceptReadWrite { .. }
        ));
        assert_eq!(
            inspect(&mut guard, &stale),
            FederationIngressOutcome::Quarantine {
                reason: FederationQuarantineReason::StateConflict,
                matching_rule_ids: vec![]
            }
        );
    }

    #[test]
    fn 对端封禁和敌对信誉都在解析正文前隔离() {
        let event_type = format!("{ACTIVE_EVENT_NAMESPACE}.message.preview.v1");
        let mut policies = FederationPolicySet::default();
        policies
            .register(
                FederationRule::new(
                    FederationRuleId::from_uuid(Uuid::from_u128(1)),
                    FederationScope::server("peer.example").expect("作用域有效"),
                    FederationDisposition::Block,
                    PrincipalId::from_uuid(Uuid::from_u128(2)),
                    time(1),
                    "应急阻断",
                    None,
                )
                .expect("规则有效"),
            )
            .expect("规则可注册");
        let mut guard = FederationIngressGuard::new(limits());
        assert!(matches!(
            guard
                .inspect(
                    &policies,
                    FederationReputation::default(),
                    &event(1, "@agent:peer.example", &event_type, 100)
                )
                .expect("检查成功"),
            FederationIngressOutcome::Reject {
                reason: FederationIngressRejection::GovernanceBlock,
                ..
            }
        ));

        let mut hostile = FederationReputation::default();
        hostile.observe(ReputationSignal::InvalidSignature);
        hostile.observe(ReputationSignal::InvalidSignature);
        assert_eq!(
            guard
                .inspect(
                    &FederationPolicySet::default(),
                    hostile,
                    &event(2, "@agent:peer.example", &event_type, 100)
                )
                .expect("检查成功"),
            FederationIngressOutcome::Quarantine {
                reason: FederationQuarantineReason::HostileReputation,
                matching_rule_ids: vec![]
            }
        );
    }
}
