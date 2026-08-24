use std::sync::Arc;

use agent_room_domain::{
    ids::RoomCatalogId,
    rooms::{RoomInstance, RoomReservation},
    time::UtcMillis,
};

use crate::ports::PortFuture;

use super::{
    JoinLobbyFailure, JoinLobbyOutcome, JoinLobbyRequest, JoinLobbyResult, JoinLobbyService,
    LobbyJoinKind, LobbyProvisioningFailure, LobbyProvisioningOutcome, LobbyProvisioningRequest,
    LobbyProvisioningResult, LobbyProvisioningService,
};

/// 大厅加入编排依赖的最小加入能力。
pub trait LobbyJoinOperation: Send + Sync {
    fn join(&self, request: JoinLobbyRequest) -> PortFuture<'_, JoinLobbyResult<JoinLobbyOutcome>>;
}

impl LobbyJoinOperation for JoinLobbyService {
    fn join(&self, request: JoinLobbyRequest) -> PortFuture<'_, JoinLobbyResult<JoinLobbyOutcome>> {
        Box::pin(async move { JoinLobbyService::join(self, request).await })
    }
}

/// 大厅加入编排依赖的最小供给能力。
pub trait LobbyProvisioningOperation: Send + Sync {
    fn provision(
        &self,
        request: LobbyProvisioningRequest,
    ) -> PortFuture<'_, LobbyProvisioningResult<LobbyProvisioningOutcome>>;
}

impl LobbyProvisioningOperation for LobbyProvisioningService {
    fn provision(
        &self,
        request: LobbyProvisioningRequest,
    ) -> PortFuture<'_, LobbyProvisioningResult<LobbyProvisioningOutcome>> {
        Box::pin(async move { LobbyProvisioningService::provision(self, request).await })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterLobbyOutcome {
    Joined {
        reservation: RoomReservation,
        room: RoomInstance,
        kind: LobbyJoinKind,
    },
    ProvisioningBusy {
        retry_at: UtcMillis,
    },
    CapacityChanged {
        catalog_id: RoomCatalogId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterLobbyFailure {
    Join(JoinLobbyFailure),
    Provisioning(LobbyProvisioningFailure),
    ProvisioningResultMismatch {
        expected_catalog_id: RoomCatalogId,
        actual_catalog_id: RoomCatalogId,
        room_catalog_id: RoomCatalogId,
    },
}

pub type EnterLobbyResult<T> = Result<T, EnterLobbyFailure>;

pub struct EnterLobbyDependencies {
    pub joins: Arc<dyn LobbyJoinOperation>,
    pub provisioning: Arc<dyn LobbyProvisioningOperation>,
}

/// 对调用方隐藏“查房、建 Space、建实例、再预约”的两阶段细节。
pub struct EnterLobbyService {
    joins: Arc<dyn LobbyJoinOperation>,
    provisioning: Arc<dyn LobbyProvisioningOperation>,
}

impl EnterLobbyService {
    pub fn new(dependencies: EnterLobbyDependencies) -> Self {
        Self {
            joins: dependencies.joins,
            provisioning: dependencies.provisioning,
        }
    }

    /// 加入可用实例；缺房时自动供给后重新预约。
    ///
    /// # Errors
    ///
    /// 分配、Matrix 成员操作、供给或供给结果不一致时返回阶段明确的错误。
    pub async fn enter(&self, request: JoinLobbyRequest) -> EnterLobbyResult<EnterLobbyOutcome> {
        let first = self
            .joins
            .join(request.clone())
            .await
            .map_err(EnterLobbyFailure::Join)?;
        let catalog = match first {
            JoinLobbyOutcome::Joined {
                reservation,
                room,
                kind,
            } => {
                return Ok(EnterLobbyOutcome::Joined {
                    reservation,
                    room,
                    kind,
                });
            }
            JoinLobbyOutcome::ProvisioningRequired { catalog } => catalog,
        };

        let expected_catalog_id = request.catalog_id;
        let provisioned = self
            .provisioning
            .provision(LobbyProvisioningRequest {
                catalog,
                preferred_region: request.preferred_region.clone(),
            })
            .await
            .map_err(EnterLobbyFailure::Provisioning)?;
        match provisioned {
            LobbyProvisioningOutcome::Busy { retry_at } => {
                Ok(EnterLobbyOutcome::ProvisioningBusy { retry_at })
            }
            LobbyProvisioningOutcome::Ready(ready) => {
                let actual_catalog_id = ready.catalog.id();
                let room_catalog_id = ready.room.catalog_id();
                if actual_catalog_id != expected_catalog_id
                    || room_catalog_id != expected_catalog_id
                {
                    return Err(EnterLobbyFailure::ProvisioningResultMismatch {
                        expected_catalog_id,
                        actual_catalog_id,
                        room_catalog_id,
                    });
                }
                self.retry_after_provisioning(request).await
            }
        }
    }

    async fn retry_after_provisioning(
        &self,
        request: JoinLobbyRequest,
    ) -> EnterLobbyResult<EnterLobbyOutcome> {
        let catalog_id = request.catalog_id;
        match self
            .joins
            .join(request)
            .await
            .map_err(EnterLobbyFailure::Join)?
        {
            JoinLobbyOutcome::Joined {
                reservation,
                room,
                kind,
            } => Ok(EnterLobbyOutcome::Joined {
                reservation,
                room,
                kind,
            }),
            JoinLobbyOutcome::ProvisioningRequired { .. } => {
                Ok(EnterLobbyOutcome::CapacityChanged { catalog_id })
            }
        }
    }
}
