use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use agent_room_application::ports::{MatrixOperation, MatrixRoomId, MatrixUserId, PortFuture};
use agent_room_bridge_core::matrix_security::{
    MatrixIdentityState, MatrixSecurityCommand, MatrixSecurityDevice, MatrixSecurityFailure,
    MatrixSecurityGateway, MatrixSecurityResult, MatrixVerificationAction,
    MatrixVerificationSnapshot, MatrixVerificationStage, MatrixVerificationTarget, identity_state,
    validate_sas_confirmation,
};
use matrix_sdk::{
    Client,
    encryption::verification::{SasVerification, VerificationRequest, VerificationRequestState},
    ruma::{
        OwnedDeviceId, RoomId, UserId,
        events::{
            StateEventType,
            key::verification::VerificationMethod,
            room::member::{MembershipState, RoomMemberEventContent},
        },
    },
};
use matrix_sdk_base::crypto::SasState;
use tokio::sync::Mutex;

const MAX_VERIFICATIONS: usize = 16;
const VERIFICATION_LIFETIME: Duration = Duration::from_mins(10);

struct ActiveVerification {
    target: MatrixVerificationTarget,
    request: VerificationRequest,
    created: Instant,
}

/// 复用 Agent 的加密 Store 和 Matrix 会话；IPC 只获得公钥状态及一次性 SAS 数字。
pub(crate) struct MatrixSdkSecurityGateway {
    client: Client,
    active: Mutex<BTreeMap<(String, String), ActiveVerification>>,
    operation: Mutex<()>,
}

impl MatrixSdkSecurityGateway {
    pub(crate) fn new(client: Client) -> Arc<Self> {
        Arc::new(Self {
            client,
            active: Mutex::new(BTreeMap::new()),
            operation: Mutex::new(()),
        })
    }

    async fn identity(&self) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        let user = self
            .client
            .user_id()
            .ok_or(MatrixSecurityFailure::Unavailable)?;
        let device = self
            .client
            .device_id()
            .ok_or(MatrixSecurityFailure::Unavailable)?;
        let crypto = self.client.encryption();
        let published = crypto
            .request_user_identity(user)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?
            .is_some();
        let keys = crypto
            .cross_signing_status()
            .await
            .is_some_and(|status| status.is_complete());
        let signed = crypto
            .get_device(user, device)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?
            .is_some_and(|device| device.is_cross_signed_by_owner());
        Ok(MatrixSecurityResult::Identity {
            user_id: user.to_string(),
            device_id: device.to_string(),
            state: identity_state(published, keys, signed),
        })
    }

    async fn establish(&self) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        match self.identity().await? {
            ready @ MatrixSecurityResult::Identity {
                state: MatrixIdentityState::Ready,
                ..
            } => return Ok(ready),
            MatrixSecurityResult::Identity {
                state: MatrixIdentityState::RecoveryRequired,
                ..
            } => return Err(MatrixSecurityFailure::RecoveryRequired),
            _ => {}
        }
        // SDK 会再次查询服务端，并且仅在不存在公开身份时创建，禁止重置现有身份。
        self.client
            .encryption()
            .bootstrap_cross_signing_if_needed(None)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
        self.identity().await
    }

    async fn ensure_peer(
        &self,
        room: &MatrixRoomId,
        user: &MatrixUserId,
    ) -> Result<(), MatrixSecurityFailure> {
        let room =
            RoomId::parse(room.as_str()).map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
        let own = self
            .client
            .user_id()
            .ok_or(MatrixSecurityFailure::Unavailable)?;
        for subject in [own.as_str(), user.as_str()] {
            let membership = crate::sdk::get_state_content::<RoomMemberEventContent>(
                &self.client,
                room.clone(),
                StateEventType::RoomMember,
                subject.to_owned(),
                MatrixOperation::InspectMembership,
            )
            .await
            .map_err(|failure| {
                if failure.kind() == agent_room_application::ports::MatrixFailureKind::Forbidden {
                    MatrixSecurityFailure::NotJoined
                } else {
                    MatrixSecurityFailure::Unavailable
                }
            })?;
            if membership.is_none_or(|member| member.membership != MembershipState::Join) {
                return Err(MatrixSecurityFailure::NotJoined);
            }
        }
        Ok(())
    }

    async fn devices(
        &self,
        room: MatrixRoomId,
        user: MatrixUserId,
    ) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        self.ensure_peer(&room, &user).await?;
        let user =
            UserId::parse(user.as_str()).map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
        let crypto = self.client.encryption();
        crypto
            .request_user_identity(&user)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
        let devices = crypto
            .get_user_devices(&user)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
        let devices = devices
            .devices()
            .take(65)
            .map(|device| MatrixSecurityDevice {
                user_id: device.user_id().to_string(),
                device_id: device.device_id().to_string(),
                owner_signed: device.is_cross_signed_by_owner(),
                verified: device.is_verified(),
            })
            .collect::<Vec<_>>();
        if devices.len() > 64 {
            return Err(MatrixSecurityFailure::InvalidRequest);
        }
        Ok(MatrixSecurityResult::Devices(devices))
    }

    async fn start(
        &self,
        room: MatrixRoomId,
        user: MatrixUserId,
        device: agent_room_application::ports::MatrixDeviceId,
    ) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        self.ensure_peer(&room, &user).await?;
        {
            let mut active = self.active.lock().await;
            active.retain(|_, entry| entry.created.elapsed() < VERIFICATION_LIFETIME);
            if active.len() >= MAX_VERIFICATIONS {
                return Err(MatrixSecurityFailure::VerificationUnavailable);
            }
        }
        let crypto = self.client.encryption();
        let user_id =
            UserId::parse(user.as_str()).map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
        crypto
            .request_user_identity(&user_id)
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
        let device = crypto
            .get_device(&user_id, &OwnedDeviceId::from(device.as_str()))
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?
            .ok_or(MatrixSecurityFailure::VerificationUnavailable)?;
        let request = device
            .request_verification_with_methods(vec![VerificationMethod::SasV1])
            .await
            .map_err(|_| MatrixSecurityFailure::Unavailable)?;
        let target = MatrixVerificationTarget {
            room_id: room,
            user_id: user,
            flow_id: request.flow_id().to_owned(),
        };
        self.active.lock().await.insert(
            (target.user_id.as_str().to_owned(), target.flow_id.clone()),
            ActiveVerification {
                target: target.clone(),
                request: request.clone(),
                created: Instant::now(),
            },
        );
        self.snapshot(target, request).await
    }

    async fn verification(
        &self,
        target: MatrixVerificationTarget,
        action: MatrixVerificationAction,
    ) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        self.ensure_peer(&target.room_id, &target.user_id).await?;
        let user = UserId::parse(target.user_id.as_str())
            .map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
        let key = (target.user_id.as_str().to_owned(), target.flow_id.clone());
        let request = {
            let mut active = self.active.lock().await;
            active.retain(|_, entry| entry.created.elapsed() < VERIFICATION_LIFETIME);
            if let Some(entry) = active.get(&key) {
                if entry.target.room_id != target.room_id {
                    return Err(MatrixSecurityFailure::InvalidRequest);
                }
                Some(entry.request.clone())
            } else {
                None
            }
        };
        let request = match request {
            Some(request) => request,
            None => self
                .client
                .encryption()
                .get_verification_request(&user, &target.flow_id)
                .await
                .ok_or(MatrixSecurityFailure::VerificationUnavailable)?,
        };
        if request.other_user_id() != user || request.is_passive() {
            return Err(MatrixSecurityFailure::VerificationUnavailable);
        }
        if let Some(room) = request.room_id()
            && room.as_str() != target.room_id.as_str()
        {
            return Err(MatrixSecurityFailure::InvalidRequest);
        }
        {
            let mut active = self.active.lock().await;
            if !active.contains_key(&key) {
                if active.len() >= MAX_VERIFICATIONS {
                    return Err(MatrixSecurityFailure::VerificationUnavailable);
                }
                active.insert(
                    key,
                    ActiveVerification {
                        target: target.clone(),
                        request: request.clone(),
                        created: Instant::now(),
                    },
                );
            }
        }
        match action {
            MatrixVerificationAction::Accept => {
                if matches!(request.state(), VerificationRequestState::Requested { .. }) {
                    request
                        .accept_with_methods(vec![VerificationMethod::SasV1])
                        .await
                        .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                    request
                        .start_sas()
                        .await
                        .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                }
            }
            MatrixVerificationAction::Poll => {}
            MatrixVerificationAction::Confirm {
                decimals,
                human_confirmed,
            } => {
                let sas = self
                    .sas(&target)
                    .await
                    .ok_or(MatrixSecurityFailure::VerificationUnavailable)?;
                let validation =
                    validate_sas_confirmation(sas_decimals(&sas), decimals, human_confirmed);
                if validation == Err(MatrixSecurityFailure::SasMismatch) {
                    sas.mismatch()
                        .await
                        .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                }
                validation?;
                sas.confirm()
                    .await
                    .map_err(|_| MatrixSecurityFailure::Unavailable)?;
            }
            MatrixVerificationAction::Mismatch => {
                self.sas(&target)
                    .await
                    .ok_or(MatrixSecurityFailure::VerificationUnavailable)?
                    .mismatch()
                    .await
                    .map_err(|_| MatrixSecurityFailure::Unavailable)?;
            }
            MatrixVerificationAction::Cancel => request
                .cancel()
                .await
                .map_err(|_| MatrixSecurityFailure::Unavailable)?,
        }
        self.snapshot(target, request).await
    }

    async fn sas(&self, target: &MatrixVerificationTarget) -> Option<SasVerification> {
        let user = UserId::parse(target.user_id.as_str()).ok()?;
        self.client
            .encryption()
            .get_verification(&user, &target.flow_id)
            .await?
            .sas()
    }

    async fn snapshot(
        &self,
        target: MatrixVerificationTarget,
        request: VerificationRequest,
    ) -> Result<MatrixSecurityResult, MatrixSecurityFailure> {
        let mut snapshot = MatrixVerificationSnapshot {
            target: target.clone(),
            device_id: None,
            stage: MatrixVerificationStage::Waiting,
            decimals: None,
        };
        snapshot.stage = match request.state() {
            VerificationRequestState::Done => MatrixVerificationStage::Verified,
            VerificationRequestState::Cancelled(_) => MatrixVerificationStage::Cancelled,
            VerificationRequestState::Requested {
                other_device_data, ..
            } => {
                snapshot.device_id = Some(other_device_data.device_id().to_string());
                MatrixVerificationStage::Requested
            }
            _ => MatrixVerificationStage::Waiting,
        };
        if let Some(sas) = self.sas(&target).await {
            // 接受已经由参与者明确开始的 SAS 协商，仍必须等待本机人类确认数字。
            if matches!(sas.state(), SasState::Started { .. }) {
                sas.accept()
                    .await
                    .map_err(|_| MatrixSecurityFailure::Unavailable)?;
            }
            snapshot.device_id = Some(sas.other_device().device_id().to_string());
            snapshot.decimals = sas_decimals(&sas);
            snapshot.stage = match sas.state() {
                SasState::Done { .. } => MatrixVerificationStage::Verified,
                SasState::Cancelled(_) => MatrixVerificationStage::Cancelled,
                SasState::KeysExchanged { .. } => MatrixVerificationStage::Comparing,
                SasState::Confirmed => MatrixVerificationStage::Confirming,
                _ => MatrixVerificationStage::Waiting,
            };
        }
        Ok(MatrixSecurityResult::Verification(snapshot))
    }
}

impl MatrixSecurityGateway for MatrixSdkSecurityGateway {
    fn ensure_room_ready<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, Result<(), MatrixSecurityFailure>> {
        Box::pin(async move {
            if !matches!(
                self.identity().await?,
                MatrixSecurityResult::Identity {
                    state: MatrixIdentityState::Ready,
                    ..
                }
            ) {
                return Err(MatrixSecurityFailure::IdentityNotReady);
            }
            let room = RoomId::parse(room_id.as_str())
                .map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
            let room = self
                .client
                .get_room(&room)
                .ok_or(MatrixSecurityFailure::NotJoined)?;
            let members = room
                .members(matrix_sdk::RoomMemberships::JOIN)
                .await
                .map_err(|_| MatrixSecurityFailure::Unavailable)?;
            let own = self
                .client
                .user_id()
                .ok_or(MatrixSecurityFailure::Unavailable)?;
            let crypto = self.client.encryption();
            for member in members {
                if member.user_id() == own {
                    continue;
                }
                let user = MatrixUserId::new(member.user_id().to_string())
                    .map_err(|_| MatrixSecurityFailure::InvalidRequest)?;
                self.ensure_peer(room_id, &user).await?;
                crypto
                    .request_user_identity(member.user_id())
                    .await
                    .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                let devices = crypto
                    .get_user_devices(member.user_id())
                    .await
                    .map_err(|_| MatrixSecurityFailure::Unavailable)?;
                if !devices
                    .devices()
                    .any(|device| device.is_verified() && device.is_cross_signed_by_owner())
                {
                    return Err(MatrixSecurityFailure::PeerVerificationRequired);
                }
            }
            Ok(())
        })
    }

    fn execute(
        &self,
        command: MatrixSecurityCommand,
    ) -> PortFuture<'_, Result<MatrixSecurityResult, MatrixSecurityFailure>> {
        Box::pin(async move {
            let _operation = self.operation.lock().await;
            match command {
                MatrixSecurityCommand::Inspect => self.identity().await,
                MatrixSecurityCommand::EstablishIdentity => self.establish().await,
                MatrixSecurityCommand::Devices { room_id, user_id } => {
                    self.devices(room_id, user_id).await
                }
                MatrixSecurityCommand::Start {
                    room_id,
                    user_id,
                    device_id,
                } => self.start(room_id, user_id, device_id).await,
                MatrixSecurityCommand::Verification { target, action } => {
                    self.verification(target, action).await
                }
            }
        })
    }
}

fn sas_decimals(sas: &SasVerification) -> Option<[u16; 3]> {
    sas.decimals().map(|(a, b, c)| [a, b, c])
}
