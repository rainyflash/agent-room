use std::sync::Arc;

use crate::{
    persistence::RepositoryError,
    ports::{
        ContentAccessMode, ContentAuthorizationDecision, ContentAuthorizationFailure,
        ContentAuthorizationFailureKind, ContentAuthorizationRequest, ContentAuthorizationResult,
        ContentMembershipAuthorizer, ContentPrincipalIdentityLookup, MatrixFailure,
        MatrixRoomAuthority, MatrixRoomAuthorityGateway, PortFuture,
    },
};

const MATRIX_MODERATOR_POWER_LEVEL: i64 = 50;

pub struct ContentMembershipAuthorizationDependencies {
    pub identities: Arc<dyn ContentPrincipalIdentityLookup>,
    pub matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
}

/// 把控制平面主体映射和 Matrix 当前状态组合成内容访问决策。
pub struct ContentMembershipAuthorizationService {
    identities: Arc<dyn ContentPrincipalIdentityLookup>,
    matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
}

impl ContentMembershipAuthorizationService {
    pub fn new(dependencies: ContentMembershipAuthorizationDependencies) -> Self {
        Self {
            identities: dependencies.identities,
            matrix_authority: dependencies.matrix_authority,
        }
    }

    async fn decide(
        &self,
        request: &ContentAuthorizationRequest,
    ) -> ContentAuthorizationResult<ContentAuthorizationDecision> {
        let Some(user_id) = self
            .identities
            .find_active_matrix_user(request.principal_id)
            .await
            .map_err(map_repository_failure)?
        else {
            return Ok(ContentAuthorizationDecision::Denied);
        };
        let authority = self
            .matrix_authority
            .inspect_room_authority(&request.matrix_room_id, &user_id)
            .await
            .map_err(map_matrix_failure)?;
        Ok(decide_access(request, authority))
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

fn decide_access(
    request: &ContentAuthorizationRequest,
    authority: MatrixRoomAuthority,
) -> ContentAuthorizationDecision {
    if !authority.is_joined() {
        return ContentAuthorizationDecision::Denied;
    }
    let allowed = match request.access_mode {
        ContentAccessMode::RoomMember => true,
        ContentAccessMode::SenderOnly => request.principal_id == request.owner_principal_id,
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

fn map_repository_failure(_failure: RepositoryError) -> ContentAuthorizationFailure {
    unavailable("content.authorization.identity")
}

fn map_matrix_failure(_failure: MatrixFailure) -> ContentAuthorizationFailure {
    unavailable("content.authorization.matrix")
}

const fn unavailable(operation: &'static str) -> ContentAuthorizationFailure {
    ContentAuthorizationFailure::new(operation, ContentAuthorizationFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_domain::ids::PrincipalId;
    use uuid::Uuid;

    use crate::{
        persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
        ports::{
            ContentAccessMode, ContentAuthorizationDecision, ContentAuthorizationFailureKind,
            ContentAuthorizationRequest, ContentMembershipAuthorizer,
            ContentPrincipalIdentityLookup, MatrixPowerLevel, MatrixResult, MatrixRoomAuthority,
            MatrixRoomAuthorityGateway, MatrixRoomId, MatrixUserId, PortFuture,
        },
    };

    use super::{
        ContentMembershipAuthorizationDependencies, ContentMembershipAuthorizationService,
    };

    struct StubIdentity {
        result: RepositoryResult<Option<MatrixUserId>>,
    }

    impl ContentPrincipalIdentityLookup for StubIdentity {
        fn find_active_matrix_user(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
            Box::pin(async move { self.result.clone() })
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
    async fn 停用主体直接拒绝且依赖错误响亮失败() {
        let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
        let denied = ContentMembershipAuthorizationService::new(
            ContentMembershipAuthorizationDependencies {
                identities: Arc::new(StubIdentity { result: Ok(None) }),
                matrix_authority: Arc::new(StubAuthority {
                    result: Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::Infinite)),
                }),
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
                    result: Err(RepositoryError::new(
                        "test.identity",
                        RepositoryErrorKind::Unavailable,
                    )),
                }),
                matrix_authority: Arc::new(StubAuthority {
                    result: Ok(MatrixRoomAuthority::not_joined()),
                }),
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
        let user_id = MatrixUserId::new(format!("@user_{}:matrix.test", Uuid::now_v7().simple()))
            .expect("用户 ID 有效");
        (
            ContentMembershipAuthorizationService::new(
                ContentMembershipAuthorizationDependencies {
                    identities: Arc::new(StubIdentity {
                        result: Ok(Some(user_id)),
                    }),
                    matrix_authority: Arc::new(StubAuthority { result: authority }),
                },
            ),
            principal_id,
        )
    }

    fn request(
        principal_id: PrincipalId,
        owner_principal_id: PrincipalId,
        access_mode: ContentAccessMode,
    ) -> ContentAuthorizationRequest {
        ContentAuthorizationRequest {
            principal_id,
            owner_principal_id,
            matrix_room_id: MatrixRoomId::new("!authorization:matrix.test").expect("房间 ID 有效"),
            access_mode,
        }
    }
}
