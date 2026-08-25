use std::sync::Arc;

use agent_room_domain::{private_rooms::PrivateRoomCapability, rooms::MatrixRoomReference};

use crate::{
    persistence::RepositoryError,
    ports::{
        ContentAccessMode, ContentAuthorizationDecision, ContentAuthorizationFailure,
        ContentAuthorizationFailureKind, ContentAuthorizationIntent, ContentAuthorizationRequest,
        ContentAuthorizationResult, ContentMembershipAuthorizer, ContentPrincipalIdentityLookup,
        DirectSessionRecord, DirectSessionStore, MatrixFailure, MatrixRoomAuthority,
        MatrixRoomAuthorityGateway, PortFuture, PrivateRoomSnapshot, PrivateRoomStore,
    },
};

const MATRIX_MODERATOR_POWER_LEVEL: i64 = 50;

pub struct ContentMembershipAuthorizationDependencies {
    pub identities: Arc<dyn ContentPrincipalIdentityLookup>,
    pub matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
    pub private_rooms: Arc<dyn PrivateRoomStore>,
    pub direct_sessions: Arc<dyn DirectSessionStore>,
}

/// 把控制平面主体映射和 Matrix 当前状态组合成内容访问决策。
pub struct ContentMembershipAuthorizationService {
    identities: Arc<dyn ContentPrincipalIdentityLookup>,
    matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
    private_rooms: Arc<dyn PrivateRoomStore>,
    direct_sessions: Arc<dyn DirectSessionStore>,
}

impl ContentMembershipAuthorizationService {
    pub fn new(dependencies: ContentMembershipAuthorizationDependencies) -> Self {
        Self {
            identities: dependencies.identities,
            matrix_authority: dependencies.matrix_authority,
            private_rooms: dependencies.private_rooms,
            direct_sessions: dependencies.direct_sessions,
        }
    }

    async fn decide(
        &self,
        request: &ContentAuthorizationRequest,
    ) -> ContentAuthorizationResult<ContentAuthorizationDecision> {
        if request.access_mode == ContentAccessMode::SenderOnly
            && request.principal_id != request.owner_principal_id
        {
            return Ok(ContentAuthorizationDecision::Denied);
        }
        let room_reference = MatrixRoomReference::new(request.matrix_room_id.as_str().to_owned())
            .map_err(|_| unavailable("content.authorization.room_id"))?;
        let private_room = self.find_private_room(&room_reference).await?;
        let direct_session = self.find_direct_session(&room_reference).await?;
        let room_policy = classify_room(private_room.as_ref(), direct_session.as_ref())?;
        if private_room
            .as_ref()
            .is_some_and(|snapshot| !private_policy_allows(request, snapshot))
        {
            return Ok(ContentAuthorizationDecision::Denied);
        }
        if let Some(record) = direct_session.as_ref()
            && !self.direct_policy_allows(request, record).await?
        {
            return Ok(ContentAuthorizationDecision::Denied);
        }
        let user_id = match request.actor_agent_id {
            Some(agent_id) => {
                self.identities
                    .find_active_agent_matrix_user(request.principal_id, agent_id)
                    .await
            }
            None => {
                self.identities
                    .find_active_matrix_user(request.principal_id)
                    .await
            }
        }
        .map_err(map_repository_failure)?;
        let Some(user_id) = user_id else {
            return Ok(ContentAuthorizationDecision::Denied);
        };
        let authority = self
            .matrix_authority
            .inspect_room_authority(&request.matrix_room_id, &user_id)
            .await
            .map_err(map_matrix_failure)?;
        Ok(decide_access(request, authority, room_policy))
    }

    async fn find_private_room(
        &self,
        room: &MatrixRoomReference,
    ) -> ContentAuthorizationResult<Option<PrivateRoomSnapshot>> {
        self.private_rooms
            .find_by_matrix_room(room)
            .await
            .map_err(map_private_room_failure)
    }

    async fn find_direct_session(
        &self,
        room: &MatrixRoomReference,
    ) -> ContentAuthorizationResult<Option<DirectSessionRecord>> {
        self.direct_sessions
            .find_by_matrix_room(room)
            .await
            .map_err(map_direct_session_failure)
    }

    async fn direct_policy_allows(
        &self,
        request: &ContentAuthorizationRequest,
        record: &DirectSessionRecord,
    ) -> ContentAuthorizationResult<bool> {
        let session = record.session();
        let is_participant = match request.actor_agent_id {
            Some(agent_id) => agent_id == session.target_agent_id(),
            None => request.principal_id == session.principal_id(),
        };
        if !is_participant || request.access_mode == ContentAccessMode::Moderator {
            return Ok(false);
        }
        if request.intent == ContentAuthorizationIntent::Read {
            return Ok(true);
        }
        self.direct_sessions
            .contact_policy(session.principal_id(), session.target_agent_id())
            .await
            .map(agent_room_domain::direct_sessions::DirectContactPolicy::delivery_allowed)
            .map_err(map_direct_session_failure)
    }
}

impl ContentMembershipAuthorizer for ContentMembershipAuthorizationService {
    fn authorize<'a>(
        &'a self,
        request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>> {
        Box::pin(async move { self.decide(request).await })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentRoomPolicy {
    Public,
    Private,
    Direct,
}

fn classify_room(
    private_room: Option<&PrivateRoomSnapshot>,
    direct_session: Option<&DirectSessionRecord>,
) -> ContentAuthorizationResult<ContentRoomPolicy> {
    match (private_room, direct_session) {
        (None, None) => Ok(ContentRoomPolicy::Public),
        (Some(_), None) => Ok(ContentRoomPolicy::Private),
        (None, Some(_)) => Ok(ContentRoomPolicy::Direct),
        (Some(_), Some(_)) => Err(unavailable("content.authorization.ambiguous_room_kind")),
    }
}

fn decide_access(
    request: &ContentAuthorizationRequest,
    authority: MatrixRoomAuthority,
    room_policy: ContentRoomPolicy,
) -> ContentAuthorizationDecision {
    if !authority.is_joined() {
        return ContentAuthorizationDecision::Denied;
    }
    let allowed = match request.access_mode {
        ContentAccessMode::RoomMember => true,
        ContentAccessMode::SenderOnly => request.principal_id == request.owner_principal_id,
        ContentAccessMode::Moderator if room_policy == ContentRoomPolicy::Private => true,
        ContentAccessMode::Moderator if room_policy == ContentRoomPolicy::Direct => false,
        ContentAccessMode::Moderator => authority
            .power_level()
            .is_at_least(MATRIX_MODERATOR_POWER_LEVEL),
    };
    if allowed {
        ContentAuthorizationDecision::Allowed
    } else {
        ContentAuthorizationDecision::Denied
    }
}

fn private_policy_allows(
    request: &ContentAuthorizationRequest,
    snapshot: &PrivateRoomSnapshot,
) -> bool {
    let room = snapshot.room();
    if !room.allows(request.principal_id, PrivateRoomCapability::View) {
        return false;
    }
    match request.intent {
        ContentAuthorizationIntent::Publish => {
            let capability = if request.actor_agent_id.is_some() {
                PrivateRoomCapability::Automate
            } else {
                PrivateRoomCapability::Speak
            };
            room.allows(request.principal_id, capability)
        }
        ContentAuthorizationIntent::Read if request.access_mode == ContentAccessMode::Moderator => {
            room.allows(request.principal_id, PrivateRoomCapability::Manage)
        }
        ContentAuthorizationIntent::Read => true,
    }
}

fn map_repository_failure(_failure: RepositoryError) -> ContentAuthorizationFailure {
    unavailable("content.authorization.identity")
}

fn map_matrix_failure(_failure: MatrixFailure) -> ContentAuthorizationFailure {
    unavailable("content.authorization.matrix")
}

fn map_private_room_failure(_failure: RepositoryError) -> ContentAuthorizationFailure {
    unavailable("content.authorization.private_room")
}

fn map_direct_session_failure(_failure: RepositoryError) -> ContentAuthorizationFailure {
    unavailable("content.authorization.direct_session")
}

const fn unavailable(operation: &'static str) -> ContentAuthorizationFailure {
    ContentAuthorizationFailure::new(operation, ContentAuthorizationFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_domain::{
        direct_sessions::{DirectContactPolicy, DirectSession},
        ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
        private_rooms::{PrivateRoom, PrivateRoomCapability, PrivateRoomPermissions},
        rooms::{
            MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
            RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
            RoomInstanceState,
        },
        time::UtcMillis,
        version::AggregateVersion,
    };
    use uuid::Uuid;

    use crate::{
        persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
        ports::{
            ContentAccessMode, ContentAuthorizationDecision, ContentAuthorizationFailureKind,
            ContentAuthorizationIntent, ContentAuthorizationRequest, ContentMembershipAuthorizer,
            ContentPrincipalIdentityLookup, DirectSessionRecord, DirectSessionStore,
            MatrixPowerLevel, MatrixResult, MatrixRoomAuthority, MatrixRoomAuthorityGateway,
            MatrixRoomId, MatrixUserId, PortFuture, PrivateRoomSnapshot, PrivateRoomStore,
        },
    };

    use super::{
        ContentMembershipAuthorizationDependencies, ContentMembershipAuthorizationService,
    };

    struct StubIdentity {
        principal_result: RepositoryResult<Option<MatrixUserId>>,
        agent_result: RepositoryResult<Option<MatrixUserId>>,
    }

    impl ContentPrincipalIdentityLookup for StubIdentity {
        fn find_active_matrix_user(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
            Box::pin(async move { self.principal_result.clone() })
        }

        fn find_active_agent_matrix_user(
            &self,
            _principal_id: PrincipalId,
            _agent_id: AgentId,
        ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
            Box::pin(async move { self.agent_result.clone() })
        }
    }

    struct StubAuthority {
        result: MatrixResult<MatrixRoomAuthority>,
    }

    impl MatrixRoomAuthorityGateway for StubAuthority {
        fn inspect_room_authority<'a>(
            &'a self,
            _room_id: &'a MatrixRoomId,
            _user_id: &'a MatrixUserId,
        ) -> PortFuture<'a, MatrixResult<MatrixRoomAuthority>> {
            Box::pin(async move { self.result })
        }
    }

    struct StubPrivateRooms {
        snapshot: Option<PrivateRoomSnapshot>,
    }

    impl PrivateRoomStore for StubPrivateRooms {
        fn create<'a>(
            &'a self,
            _snapshot: &'a PrivateRoomSnapshot,
            _created_at: UtcMillis,
        ) -> PortFuture<'a, RepositoryResult<()>> {
            Box::pin(async { unreachable!("授权只读取私人房间") })
        }

        fn find_by_catalog(
            &self,
            _catalog_id: RoomCatalogId,
        ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
            Box::pin(async { unreachable!("授权按 Matrix 房间读取") })
        }

        fn list_for_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
            Box::pin(async { unreachable!("授权不列举私人房间") })
        }

        fn find_by_matrix_room<'a>(
            &'a self,
            _matrix_room_id: &'a MatrixRoomReference,
        ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
            Box::pin(async move { Ok(self.snapshot.clone()) })
        }

        fn save<'a>(
            &'a self,
            _room: &'a PrivateRoom,
            _expected_version: AggregateVersion,
            _changed_at: UtcMillis,
        ) -> PortFuture<'a, RepositoryResult<()>> {
            Box::pin(async { unreachable!("授权不写私人房间") })
        }
    }

    struct StubDirectSessions {
        record: Option<DirectSessionRecord>,
        policy: DirectContactPolicy,
    }

    impl DirectSessionStore for StubDirectSessions {
        fn reserve<'a>(
            &'a self,
            _record: &'a DirectSessionRecord,
            _created_at: UtcMillis,
        ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>> {
            Box::pin(async { unreachable!("授权不预留直接会话") })
        }

        fn activate<'a>(
            &'a self,
            _record: &'a DirectSessionRecord,
            _expected_version: AggregateVersion,
            _changed_at: UtcMillis,
        ) -> PortFuture<'a, RepositoryResult<DirectSessionRecord>> {
            Box::pin(async { unreachable!("授权不激活直接会话") })
        }

        fn find_by_participants(
            &self,
            _principal_id: PrincipalId,
            _agent_id: AgentId,
        ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>> {
            Box::pin(async { unreachable!("授权按 Matrix 房间读取直接会话") })
        }

        fn find_by_catalog(
            &self,
            _catalog_id: RoomCatalogId,
        ) -> PortFuture<'_, RepositoryResult<Option<DirectSessionRecord>>> {
            Box::pin(async { unreachable!("授权不按目录读取直接会话") })
        }

        fn find_by_matrix_room<'a>(
            &'a self,
            _matrix_room_id: &'a MatrixRoomReference,
        ) -> PortFuture<'a, RepositoryResult<Option<DirectSessionRecord>>> {
            Box::pin(async move { Ok(self.record.clone()) })
        }

        fn list_for_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, RepositoryResult<Vec<DirectSessionRecord>>> {
            Box::pin(async { unreachable!("授权不列举直接会话") })
        }

        fn contact_policy(
            &self,
            _principal_id: PrincipalId,
            _agent_id: AgentId,
        ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>> {
            Box::pin(async move { Ok(self.policy) })
        }

        fn set_principal_block(
            &self,
            _principal_id: PrincipalId,
            _agent_id: AgentId,
            _blocked: bool,
            _changed_at: UtcMillis,
        ) -> PortFuture<'_, RepositoryResult<DirectContactPolicy>> {
            Box::pin(async { unreachable!("授权不修改屏蔽策略") })
        }
    }

    #[tokio::test]
    async fn 所有访问模式都先要求当前仍在房间() {
        for access_mode in [
            ContentAccessMode::RoomMember,
            ContentAccessMode::SenderOnly,
            ContentAccessMode::Moderator,
        ] {
            let (service, principal_id) = service(Ok(MatrixRoomAuthority::not_joined()));
            let decision = service
                .authorize(&request(principal_id, principal_id, access_mode))
                .await
                .expect("拒绝决策不应伪装成依赖失败");
            assert_eq!(decision, ContentAuthorizationDecision::Denied);
        }
    }

    #[tokio::test]
    async fn 发送者私有内容只允许仍在房间的原所有者() {
        let (service, principal_id) =
            service(Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::finite(0))));
        let other = PrincipalId::from_uuid(Uuid::now_v7());
        assert_eq!(
            service
                .authorize(&request(
                    principal_id,
                    principal_id,
                    ContentAccessMode::SenderOnly,
                ))
                .await
                .expect("所有者授权可判定"),
            ContentAuthorizationDecision::Allowed
        );
        assert_eq!(
            service
                .authorize(&request(principal_id, other, ContentAccessMode::SenderOnly))
                .await
                .expect("非所有者授权可判定"),
            ContentAuthorizationDecision::Denied
        );
    }

    #[tokio::test]
    async fn 主体持有的活跃_agent_入房即可代表主体访问内容() {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let agent_id = AgentId::from_uuid(Uuid::now_v7());
        let agent_user_id =
            MatrixUserId::new("@agent_owned:matrix.test").expect("Agent 用户 ID 有效");
        let service = ContentMembershipAuthorizationService::new(
            ContentMembershipAuthorizationDependencies {
                identities: Arc::new(StubIdentity {
                    principal_result: Ok(None),
                    agent_result: Ok(Some(agent_user_id)),
                }),
                matrix_authority: Arc::new(StubAuthority {
                    result: Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::finite(0))),
                }),
                private_rooms: public_room_store(),
                direct_sessions: public_direct_store(),
            },
        );

        assert_eq!(
            service
                .authorize(&agent_request(
                    principal_id,
                    agent_id,
                    principal_id,
                    ContentAccessMode::RoomMember,
                ))
                .await
                .expect("Agent 成员资格可判定"),
            ContentAuthorizationDecision::Allowed
        );
    }

    #[tokio::test]
    async fn 管理员门槛使用_matrix_标准_50_并支持无限级别() {
        for (power_level, expected) in [
            (
                MatrixPowerLevel::finite(49),
                ContentAuthorizationDecision::Denied,
            ),
            (
                MatrixPowerLevel::finite(50),
                ContentAuthorizationDecision::Allowed,
            ),
            (
                MatrixPowerLevel::Infinite,
                ContentAuthorizationDecision::Allowed,
            ),
        ] {
            let (service, principal_id) = service(Ok(MatrixRoomAuthority::joined(power_level)));
            assert_eq!(
                service
                    .authorize(&request(
                        principal_id,
                        principal_id,
                        ContentAccessMode::Moderator,
                    ))
                    .await
                    .expect("管理员授权可判定"),
                expected
            );
        }
    }

    #[tokio::test]
    async fn 私人房间移除事实会覆盖尚未收敛的_matrix_成员投影() {
        let owner = PrincipalId::from_uuid(Uuid::now_v7());
        let member = PrincipalId::from_uuid(Uuid::now_v7());
        let mut room = joined_private_room(owner, member, viewer_permissions());
        room.remove_member(owner, member).expect("房主可移除成员");
        let service = private_service(
            Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::Infinite)),
            private_snapshot(room),
        );

        assert_eq!(
            service
                .authorize(&request(member, member, ContentAccessMode::RoomMember,))
                .await
                .expect("移除事实可判定"),
            ContentAuthorizationDecision::Denied
        );
    }

    #[tokio::test]
    async fn 私人房间读取发言治理和自动发送使用产品权限与_matrix_成员交集() {
        let owner = PrincipalId::from_uuid(Uuid::now_v7());
        let member = PrincipalId::from_uuid(Uuid::now_v7());
        let viewer = private_service(
            Ok(joined_authority()),
            private_snapshot(joined_private_room(owner, member, viewer_permissions())),
        );
        assert_eq!(
            viewer
                .authorize(&request(member, member, ContentAccessMode::RoomMember,))
                .await
                .expect("查看权限可判定"),
            ContentAuthorizationDecision::Allowed
        );
        assert_eq!(
            viewer
                .authorize(&publish_request(member, None))
                .await
                .expect("发言权限可判定"),
            ContentAuthorizationDecision::Denied
        );

        let manager = private_service(
            Ok(joined_authority()),
            private_snapshot(joined_private_room(owner, member, manager_permissions())),
        );
        assert_eq!(
            manager
                .authorize(&request(member, member, ContentAccessMode::Moderator,))
                .await
                .expect("私人治理权限可判定"),
            ContentAuthorizationDecision::Allowed,
            "私人治理依赖产品权限，不应要求给用户 Matrix 管理员权限"
        );

        let agent_id = AgentId::from_uuid(Uuid::now_v7());
        let speaker = private_service(
            Ok(joined_authority()),
            private_snapshot(joined_private_room(owner, member, speaker_permissions())),
        );
        assert_eq!(
            speaker
                .authorize(&publish_request(member, Some(agent_id)))
                .await
                .expect("Agent 自动发送权限可判定"),
            ContentAuthorizationDecision::Denied
        );
        let automated = private_service(
            Ok(joined_authority()),
            private_snapshot(joined_private_room(owner, member, automated_permissions())),
        );
        assert_eq!(
            automated
                .authorize(&publish_request(member, Some(agent_id)))
                .await
                .expect("Agent 自动发送授权可判定"),
            ContentAuthorizationDecision::Allowed
        );
    }

    #[tokio::test]
    async fn 直接会话屏蔽只停止新投递且历史内容仍可读取() {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let target_agent_id = AgentId::from_uuid(Uuid::now_v7());
        let service = direct_service(
            principal_id,
            target_agent_id,
            DirectContactPolicy::restore(principal_id, target_agent_id, true, false),
        );

        assert_eq!(
            service
                .authorize(&publish_request(principal_id, None))
                .await
                .expect("屏蔽后的发布策略可判定"),
            ContentAuthorizationDecision::Denied
        );
        assert_eq!(
            service
                .authorize(&request(
                    principal_id,
                    principal_id,
                    ContentAccessMode::RoomMember,
                ))
                .await
                .expect("屏蔽后的历史读取策略可判定"),
            ContentAuthorizationDecision::Allowed
        );
    }

    #[tokio::test]
    async fn 直接会话拒绝第三方_agent_和治理者权限() {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let target_agent_id = AgentId::from_uuid(Uuid::now_v7());
        let service = direct_service(
            principal_id,
            target_agent_id,
            DirectContactPolicy::new(principal_id, target_agent_id),
        );
        let unrelated_agent_id = AgentId::from_uuid(Uuid::now_v7());

        assert_eq!(
            service
                .authorize(&publish_request(principal_id, Some(unrelated_agent_id)))
                .await
                .expect("第三方 Agent 策略可判定"),
            ContentAuthorizationDecision::Denied
        );
        assert_eq!(
            service
                .authorize(&request(
                    principal_id,
                    principal_id,
                    ContentAccessMode::Moderator,
                ))
                .await
                .expect("直接会话治理策略可判定"),
            ContentAuthorizationDecision::Denied
        );
    }

    #[tokio::test]
    async fn 停用主体直接拒绝且依赖错误响亮失败() {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let denied = ContentMembershipAuthorizationService::new(
            ContentMembershipAuthorizationDependencies {
                identities: Arc::new(StubIdentity {
                    principal_result: Ok(None),
                    agent_result: Ok(None),
                }),
                matrix_authority: Arc::new(StubAuthority {
                    result: Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::Infinite)),
                }),
                private_rooms: public_room_store(),
                direct_sessions: public_direct_store(),
            },
        );
        assert_eq!(
            denied
                .authorize(&request(
                    principal_id,
                    principal_id,
                    ContentAccessMode::RoomMember,
                ))
                .await
                .expect("停用主体可判定"),
            ContentAuthorizationDecision::Denied
        );

        let failed = ContentMembershipAuthorizationService::new(
            ContentMembershipAuthorizationDependencies {
                identities: Arc::new(StubIdentity {
                    principal_result: Err(RepositoryError::new(
                        "test.identity",
                        RepositoryErrorKind::Unavailable,
                    )),
                    agent_result: Ok(None),
                }),
                matrix_authority: Arc::new(StubAuthority {
                    result: Ok(MatrixRoomAuthority::not_joined()),
                }),
                private_rooms: public_room_store(),
                direct_sessions: public_direct_store(),
            },
        );
        let failure = failed
            .authorize(&request(
                principal_id,
                principal_id,
                ContentAccessMode::RoomMember,
            ))
            .await
            .expect_err("依赖错误不能降级成普通拒绝");
        assert_eq!(failure.kind(), ContentAuthorizationFailureKind::Unavailable);
    }

    fn service(
        authority: MatrixResult<MatrixRoomAuthority>,
    ) -> (ContentMembershipAuthorizationService, PrincipalId) {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        (
            service_with_store(authority, public_room_store()),
            principal_id,
        )
    }

    fn private_service(
        authority: MatrixResult<MatrixRoomAuthority>,
        snapshot: PrivateRoomSnapshot,
    ) -> ContentMembershipAuthorizationService {
        service_with_store(
            authority,
            Arc::new(StubPrivateRooms {
                snapshot: Some(snapshot),
            }),
        )
    }

    fn service_with_store(
        authority: MatrixResult<MatrixRoomAuthority>,
        private_rooms: Arc<dyn PrivateRoomStore>,
    ) -> ContentMembershipAuthorizationService {
        let user_id = MatrixUserId::new(format!("@user_{}:matrix.test", Uuid::now_v7().simple()))
            .expect("用户 ID 有效");
        ContentMembershipAuthorizationService::new(ContentMembershipAuthorizationDependencies {
            identities: Arc::new(StubIdentity {
                principal_result: Ok(Some(user_id.clone())),
                agent_result: Ok(Some(user_id)),
            }),
            matrix_authority: Arc::new(StubAuthority { result: authority }),
            private_rooms,
            direct_sessions: public_direct_store(),
        })
    }

    fn direct_service(
        principal_id: PrincipalId,
        target_agent_id: AgentId,
        policy: DirectContactPolicy,
    ) -> ContentMembershipAuthorizationService {
        let user_id = MatrixUserId::new("@direct_member:matrix.test").expect("用户 ID 有效");
        ContentMembershipAuthorizationService::new(ContentMembershipAuthorizationDependencies {
            identities: Arc::new(StubIdentity {
                principal_result: Ok(Some(user_id.clone())),
                agent_result: Ok(Some(user_id)),
            }),
            matrix_authority: Arc::new(StubAuthority {
                result: Ok(joined_authority()),
            }),
            private_rooms: public_room_store(),
            direct_sessions: Arc::new(StubDirectSessions {
                record: Some(direct_record(principal_id, target_agent_id)),
                policy,
            }),
        })
    }

    fn request(
        principal_id: PrincipalId,
        owner_principal_id: PrincipalId,
        access_mode: ContentAccessMode,
    ) -> ContentAuthorizationRequest {
        ContentAuthorizationRequest {
            principal_id,
            actor_agent_id: None,
            owner_principal_id,
            matrix_room_id: MatrixRoomId::new("!authorization:matrix.test").expect("房间 ID 有效"),
            access_mode,
            intent: ContentAuthorizationIntent::Read,
        }
    }

    fn agent_request(
        principal_id: PrincipalId,
        actor_agent_id: AgentId,
        owner_principal_id: PrincipalId,
        access_mode: ContentAccessMode,
    ) -> ContentAuthorizationRequest {
        ContentAuthorizationRequest {
            actor_agent_id: Some(actor_agent_id),
            ..request(principal_id, owner_principal_id, access_mode)
        }
    }

    fn publish_request(
        principal_id: PrincipalId,
        actor_agent_id: Option<AgentId>,
    ) -> ContentAuthorizationRequest {
        ContentAuthorizationRequest {
            principal_id,
            actor_agent_id,
            owner_principal_id: principal_id,
            matrix_room_id: MatrixRoomId::new("!authorization:matrix.test").expect("房间 ID 有效"),
            access_mode: ContentAccessMode::RoomMember,
            intent: ContentAuthorizationIntent::Publish,
        }
    }

    fn joined_private_room(
        owner: PrincipalId,
        member: PrincipalId,
        permissions: PrivateRoomPermissions,
    ) -> PrivateRoom {
        let mut room = PrivateRoom::create(RoomCatalogId::from_uuid(Uuid::now_v7()), owner);
        room.invite(owner, member, permissions).expect("房主可邀请");
        room.accept_invitation(member).expect("成员可接受邀请");
        room
    }

    fn private_snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
        let catalog_id = room.catalog_id();
        let owner = room.owner_principal_id();
        let catalog = RoomCatalog::new(
            catalog_id,
            RoomCatalogFields {
                kind: RoomCatalogKind::PrivateRoom,
                slug: None,
                name: "Private authorization".to_owned(),
                description: String::new(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(owner),
                visibility: RoomCatalogVisibility::Private,
                retention_days: Some(30),
                status: RoomCatalogStatus::Active,
            },
        )
        .expect("私人目录有效");
        let instance = RoomInstance::restore(
            RoomInstanceId::from_uuid(Uuid::now_v7()),
            RoomInstanceFields {
                catalog_id,
                matrix_room_id: MatrixRoomReference::new("!authorization:matrix.test".to_owned())
                    .expect("Matrix 房间标识有效"),
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: 2,
                allocated_slots: 0,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .expect("私人房间实例有效");
        PrivateRoomSnapshot::new(catalog, instance, room).expect("私人房间快照有效")
    }

    fn direct_record(principal_id: PrincipalId, target_agent_id: AgentId) -> DirectSessionRecord {
        let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
        let catalog = RoomCatalog::new(
            catalog_id,
            RoomCatalogFields {
                kind: RoomCatalogKind::Direct,
                slug: None,
                name: "Direct authorization".to_owned(),
                description: String::new(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(principal_id),
                visibility: RoomCatalogVisibility::Private,
                retention_days: None,
                status: RoomCatalogStatus::Frozen,
            },
        )
        .expect("直接会话目录有效");
        DirectSessionRecord::new(
            catalog,
            None,
            DirectSession::reserve(catalog_id, principal_id, target_agent_id),
        )
        .expect("直接会话预留记录有效")
        .activate(
            RoomInstanceId::from_uuid(Uuid::now_v7()),
            MatrixRoomReference::new("!authorization:matrix.test".to_owned())
                .expect("Matrix 房间标识有效"),
        )
        .expect("直接会话记录可激活")
    }

    fn viewer_permissions() -> PrivateRoomPermissions {
        permissions([PrivateRoomCapability::View])
    }

    fn speaker_permissions() -> PrivateRoomPermissions {
        permissions([PrivateRoomCapability::View, PrivateRoomCapability::Speak])
    }

    fn manager_permissions() -> PrivateRoomPermissions {
        permissions([PrivateRoomCapability::View, PrivateRoomCapability::Manage])
    }

    fn automated_permissions() -> PrivateRoomPermissions {
        permissions([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Automate,
        ])
    }

    fn permissions(
        capabilities: impl IntoIterator<Item = PrivateRoomCapability>,
    ) -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities(capabilities).expect("私人房间权限有效")
    }

    fn joined_authority() -> MatrixRoomAuthority {
        MatrixRoomAuthority::joined(MatrixPowerLevel::finite(0))
    }

    fn public_room_store() -> Arc<dyn PrivateRoomStore> {
        Arc::new(StubPrivateRooms { snapshot: None })
    }

    fn public_direct_store() -> Arc<dyn DirectSessionStore> {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let agent_id = AgentId::from_uuid(Uuid::now_v7());
        Arc::new(StubDirectSessions {
            record: None,
            policy: DirectContactPolicy::new(principal_id, agent_id),
        })
    }
}
