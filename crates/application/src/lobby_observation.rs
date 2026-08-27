use std::sync::Arc;

use agent_room_domain::ids::RoomCatalogId;

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{PortFuture, PublicLobbyObservationRoom, RoomDirectory},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicLobbyObservationFailureKind {
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicLobbyObservationFailure {
    operation: &'static str,
    kind: PublicLobbyObservationFailureKind,
}

impl PublicLobbyObservationFailure {
    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PublicLobbyObservationFailureKind {
        self.kind
    }
}

pub type PublicLobbyObservationResult<T> = Result<T, PublicLobbyObservationFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvePublicLobbyObservation {
    pub catalog_id: RoomCatalogId,
}

pub trait PublicLobbyObservationUseCases: Send + Sync {
    fn resolve(
        &self,
        request: ResolvePublicLobbyObservation,
    ) -> PortFuture<'_, PublicLobbyObservationResult<PublicLobbyObservationRoom>>;
}

pub struct PublicLobbyObservationService {
    directory: Arc<dyn RoomDirectory>,
}

impl PublicLobbyObservationService {
    pub fn new(directory: Arc<dyn RoomDirectory>) -> Self {
        Self { directory }
    }

    async fn resolve_internal(
        &self,
        request: ResolvePublicLobbyObservation,
    ) -> PublicLobbyObservationResult<PublicLobbyObservationRoom> {
        const OPERATION: &str = "public_lobby_observation.resolve";
        self.directory
            .find_public_observation_room(request.catalog_id)
            .await
            .map_err(|error| map_repository_failure(OPERATION, &error))?
            .ok_or(PublicLobbyObservationFailure {
                operation: OPERATION,
                kind: PublicLobbyObservationFailureKind::NotFound,
            })
    }
}

impl PublicLobbyObservationUseCases for PublicLobbyObservationService {
    fn resolve(
        &self,
        request: ResolvePublicLobbyObservation,
    ) -> PortFuture<'_, PublicLobbyObservationResult<PublicLobbyObservationRoom>> {
        Box::pin(async move { self.resolve_internal(request).await })
    }
}

fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> PublicLobbyObservationFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => {
            PublicLobbyObservationFailureKind::DependencyUnavailable
        }
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::NotFound
        | RepositoryErrorKind::CorruptData => PublicLobbyObservationFailureKind::Internal,
    };
    PublicLobbyObservationFailure { operation, kind }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_domain::{
        ids::{RoomCatalogId, RoomInstanceId},
        rooms::{MatrixRoomReference, RoomCatalog},
    };
    use uuid::Uuid;

    use super::{
        PublicLobbyObservationFailureKind, PublicLobbyObservationService,
        PublicLobbyObservationUseCases, ResolvePublicLobbyObservation,
    };
    use crate::{
        persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
        ports::{
            PortFuture, PublicLobbyDirectoryEntry, PublicLobbyObservationRoom, RoomDirectory,
            RoomDirectoryQuery,
        },
    };

    const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";

    struct FakeDirectory {
        result: RepositoryResult<Option<PublicLobbyObservationRoom>>,
    }

    impl RoomDirectory for FakeDirectory {
        fn list_public<'a>(
            &'a self,
            _query: &'a RoomDirectoryQuery,
        ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
            Box::pin(async { unreachable!("解析房间不会读取目录列表") })
        }

        fn find_catalog(
            &self,
            _catalog_id: RoomCatalogId,
        ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
            Box::pin(async { unreachable!("解析房间不会重复读取目录实体") })
        }

        fn find_public_observation_room(
            &self,
            _catalog_id: RoomCatalogId,
        ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
            let result = self.result.clone();
            Box::pin(async move { result })
        }
    }

    #[tokio::test]
    async fn 只返回存储层确认的真实活跃房间() {
        let expected = observation_room();
        let service = PublicLobbyObservationService::new(Arc::new(FakeDirectory {
            result: Ok(Some(expected.clone())),
        }));

        let result = service
            .resolve(ResolvePublicLobbyObservation {
                catalog_id: catalog_id(),
            })
            .await;

        assert_eq!(result, Ok(expected));
    }

    #[tokio::test]
    async fn 没有活跃实例时明确返回不存在而不是伪造房间() {
        let service =
            PublicLobbyObservationService::new(Arc::new(FakeDirectory { result: Ok(None) }));

        let failure = service
            .resolve(ResolvePublicLobbyObservation {
                catalog_id: catalog_id(),
            })
            .await
            .expect_err("没有活跃实例必须失败关闭");

        assert_eq!(failure.kind(), PublicLobbyObservationFailureKind::NotFound);
    }

    #[tokio::test]
    async fn 存储不可用保持可辨识依赖故障() {
        let service = PublicLobbyObservationService::new(Arc::new(FakeDirectory {
            result: Err(RepositoryError::new(
                "room_directory.find_public_observation_room",
                RepositoryErrorKind::Unavailable,
            )),
        }));

        let failure = service
            .resolve(ResolvePublicLobbyObservation {
                catalog_id: catalog_id(),
            })
            .await
            .expect_err("依赖故障不能降级为不存在");

        assert_eq!(
            failure.kind(),
            PublicLobbyObservationFailureKind::DependencyUnavailable
        );
    }

    fn observation_room() -> PublicLobbyObservationRoom {
        PublicLobbyObservationRoom {
            catalog_id: catalog_id(),
            room_instance_id: RoomInstanceId::from_uuid(uuid(INSTANCE_ID)),
            matrix_room_id: MatrixRoomReference::new(
                "!public-lobby:matrix.agent-room.test".to_owned(),
            )
            .expect("测试 Matrix 房间有效"),
        }
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(uuid(CATALOG_ID))
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }
}
