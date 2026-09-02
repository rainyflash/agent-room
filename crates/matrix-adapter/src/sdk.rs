use std::{num::NonZeroU16, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixBackfillPage, MatrixBackfillRequest, MatrixClientFactory,
    MatrixConnection, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId, MatrixFailure,
    MatrixFailureKind, MatrixGateway, MatrixLogin, MatrixOperation, MatrixPowerLevel,
    MatrixReceipt, MatrixReceiptKind, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomAuthority,
    MatrixRoomAuthorityGateway, MatrixRoomEncryption, MatrixRoomId, MatrixRoomKind,
    MatrixRoomPreset, MatrixRoomVisibility, MatrixSession, MatrixSessionMetadata, MatrixStateEvent,
    MatrixSyncBatch, MatrixSyncRequest, MatrixUserId, PortFuture, SecretValue,
};
use agent_room_bridge_core::handoffs::{
    EncryptedHandoffToDeviceEventSource, EncryptedHandoffToDeviceGateway,
};
use matrix_sdk::{
    Client, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession as SdkMatrixSession,
    config::{RequestConfig, SyncSettings, SyncToken},
    room::MessagesOptions,
    ruma::{
        OwnedDeviceId, OwnedEventId, OwnedTransactionId, OwnedUserId, RoomAliasId, RoomId, UInt,
        UserId,
        api::client::{
            filter::FilterDefinition,
            receipt::create_receipt::v3::ReceiptType,
            room::{
                Visibility,
                create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            },
            session::login::v3::{LoginInfo, Password, Request as LoginRequest},
            state::get_state_event_for_key::v3::{
                Request as GetStateEventRequest, StateEventFormat,
            },
            sync::sync_events::v3::Filter as SyncFilter,
            uiaa::{MatrixUserIdentifier, UserIdentifier},
        },
        events::{
            InitialStateEvent, StateEventType, TimelineEventType,
            receipt::ReceiptThread,
            room::{
                encryption::RoomEncryptionEventContent,
                member::{MembershipState, RoomMemberEventContent},
                power_levels::RoomPowerLevelsEventContent,
            },
        },
        room::RoomType,
        serde::Raw,
    },
    store::RoomLoadSettings,
};
use matrix_sdk_base::{
    crypto::{CollectStrategy, DecryptionSettings, TrustRequirement},
    store::StateStoreDataKey,
};

use crate::{
    configuration::{MatrixSdkConfiguration, MatrixSdkStoreConfiguration},
    error::{map_build_error, map_http_error, map_sdk_error},
    handoff::MatrixSdkHandoffGateway,
    mapping::{map_backfill, map_sync_response},
    store_recovery::{
        quarantine_invalid_state_cache, quarantine_session_store, recover_query_statistics,
    },
};

#[derive(Debug, Clone)]
pub struct MatrixSdkClientFactory {
    configuration: MatrixSdkConfiguration,
    store: MatrixSdkStore,
}

impl MatrixSdkClientFactory {
    pub const fn new(configuration: MatrixSdkConfiguration) -> Self {
        Self {
            configuration,
            store: MatrixSdkStore::Memory,
        }
    }

    pub const fn with_encrypted_sqlite(
        configuration: MatrixSdkConfiguration,
        store: MatrixSdkStoreConfiguration,
    ) -> Self {
        Self {
            configuration,
            store: MatrixSdkStore::EncryptedSqlite(store),
        }
    }

    /// 打开并校验配置的 Store，不建立网络会话。
    ///
    /// # Errors
    ///
    /// `SQLite` Store 无法创建、解密或迁移时返回 Matrix 基础设施错误。
    pub async fn initialize_store(&self) -> MatrixResult<()> {
        let first_attempt = self.build_client(MatrixOperation::InitializeStore).await;
        match first_attempt {
            Ok(client) => {
                drop(client);
                Ok(())
            }
            Err(failure) if failure.kind() == MatrixFailureKind::DependencyUnavailable => {
                let recovered = match &self.store {
                    MatrixSdkStore::Memory => false,
                    MatrixSdkStore::EncryptedSqlite(store) => {
                        recover_query_statistics(store.path()).unwrap_or(false)
                    }
                };
                if !recovered {
                    return Err(failure);
                }
                drop(self.build_client(MatrixOperation::InitializeStore).await?);
                Ok(())
            }
            Err(failure) => Err(failure),
        }
    }

    /// 恢复同一个 Matrix SDK 客户端，并额外暴露加密 To-Device 交付能力。
    ///
    /// # Errors
    ///
    /// SDK Store、会话或加密状态无法恢复时返回 Matrix 基础设施错误。
    pub async fn restore_with_handoffs(
        &self,
        session: &MatrixSession,
    ) -> MatrixResult<MatrixSdkHandoffConnection> {
        let client = self.restore_client(session).await?;
        handoff_connection_from_client(client, self.configuration.sync_timeline_limit())
    }

    /// 隔离与当前 Matrix 设备加密身份绑定的全部可再生本地 Store。
    ///
    /// 调用方必须先释放所有由本工厂创建的客户端连接。原文件会保留在恢复目录，
    /// 不执行不可逆删除。
    ///
    /// # Errors
    ///
    /// Store 不在磁盘上，或文件无法原子移动时返回 Matrix 基础设施错误。
    pub fn quarantine_device_session_store(&self) -> MatrixResult<bool> {
        let MatrixSdkStore::EncryptedSqlite(store) = &self.store else {
            return Ok(false);
        };
        quarantine_session_store(store.path()).map_err(|_| {
            MatrixFailure::new(
                MatrixOperation::InitializeStore,
                MatrixFailureKind::DependencyUnavailable,
            )
        })
    }

    async fn build_client(&self, operation: MatrixOperation) -> MatrixResult<Client> {
        let builder = Client::builder()
            .homeserver_url(self.configuration.homeserver_url().clone())
            .request_config(self.request_config())
            .with_room_key_recipient_strategy(CollectStrategy::OnlyTrustedDevices)
            // 自定义 Olm To-Device 交接必须先进入应用层，才能按控制面精确设备映射和
            // Agent Room 签名进行验证。房间事件的交叉签名信任在 mapping 适配器中保留，
            // 不允许这个 SDK 级兼容设置把未可信房间消息抬高为可信消息。
            .with_decryption_settings(DecryptionSettings {
                sender_device_trust_requirement: TrustRequirement::Untrusted,
            });
        let builder = match &self.store {
            MatrixSdkStore::Memory => builder,
            MatrixSdkStore::EncryptedSqlite(store) => {
                builder.sqlite_store(store.path(), Some(store.passphrase().expose()))
            }
        };
        builder
            .build()
            .await
            .map_err(|error| map_build_error(operation, &error))
    }

    async fn restore_client(&self, session: &MatrixSession) -> MatrixResult<Client> {
        let client = self.build_client(MatrixOperation::RestoreSession).await?;
        // 必须在 restore_session 启动长期 E2EE 任务前检查持久冲突标记。
        // Windows 不允许移动仍被这些任务持有的 SQLite 文件。
        reject_recorded_crypto_identity_conflict(&client, MatrixOperation::RestoreSession).await?;
        let sdk_session = to_sdk_session(session)?;
        let first_attempt = client
            .matrix_auth()
            .restore_session(sdk_session, RoomLoadSettings::default())
            .await;
        let Err(error) = first_attempt else {
            return Ok(client);
        };
        if !is_rebuildable_state_cache_error(&error) {
            return Err(map_sdk_error(MatrixOperation::RestoreSession, &error));
        }
        drop(client);
        let recovered = match &self.store {
            MatrixSdkStore::Memory => false,
            MatrixSdkStore::EncryptedSqlite(store) => quarantine_invalid_state_cache(store.path())
                .map_err(|_| {
                    MatrixFailure::new(
                        MatrixOperation::RestoreSession,
                        MatrixFailureKind::DependencyUnavailable,
                    )
                })?,
        };
        if !recovered {
            return Err(map_sdk_error(MatrixOperation::RestoreSession, &error));
        }

        let client = self.build_client(MatrixOperation::RestoreSession).await?;
        let sdk_session = to_sdk_session(session)?;
        client
            .matrix_auth()
            .restore_session(sdk_session, RoomLoadSettings::default())
            .await
            .map_err(|error| map_sdk_error(MatrixOperation::RestoreSession, &error))?;
        Ok(client)
    }

    fn request_config(&self) -> RequestConfig {
        RequestConfig::new()
            .disable_retry()
            .timeout(self.configuration.request_timeout())
    }
}

/// Matrix SDK 会在首次发现重复一次性密钥时把冲突写入 State Store，后续启动
/// 不再重复广播同一诊断。恢复会话时必须读取这个持久标记，否则客户端会永久
/// 卡在无意义的 `/keys/upload` 重试中，且应用层永远看不到原始 400 响应。
async fn reject_recorded_crypto_identity_conflict(
    client: &Client,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    let recorded = client
        .state_store()
        .get_kv_data(StateStoreDataKey::OneTimeKeyAlreadyUploaded)
        .await
        .map_err(|_| MatrixFailure::new(operation, MatrixFailureKind::DependencyUnavailable))?;
    if recorded.is_some() {
        return Err(MatrixFailure::new(
            operation,
            MatrixFailureKind::CryptographicIdentityConflict,
        ));
    }
    Ok(())
}

fn is_rebuildable_state_cache_error(error: &matrix_sdk::Error) -> bool {
    matches!(
        error,
        matrix_sdk::Error::StateStore(failure)
            if matches!(
                failure.as_ref(),
                matrix_sdk_base::StoreError::Encryption(_)
                    | matrix_sdk_base::StoreError::Codec(_)
                    | matrix_sdk_base::StoreError::InvalidData { .. }
            )
    )
}

pub struct MatrixSdkHandoffConnection {
    matrix: MatrixConnection,
    handoff: Arc<MatrixSdkHandoffGateway>,
}

impl std::fmt::Debug for MatrixSdkHandoffConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixSdkHandoffConnection")
            .field("matrix", &self.matrix)
            .finish_non_exhaustive()
    }
}

impl MatrixSdkHandoffConnection {
    pub const fn matrix(&self) -> &MatrixConnection {
        &self.matrix
    }

    pub fn matrix_gateway_handle(&self) -> Arc<dyn MatrixGateway> {
        self.matrix.gateway_handle()
    }

    pub fn room_authority_gateway_handle(&self) -> Arc<dyn MatrixRoomAuthorityGateway> {
        self.matrix.room_authority_gateway_handle()
    }

    pub fn handoff_transport_handle(&self) -> Arc<dyn EncryptedHandoffToDeviceGateway> {
        self.handoff.clone()
    }

    pub fn handoff_event_source_handle(&self) -> Arc<dyn EncryptedHandoffToDeviceEventSource> {
        self.handoff.clone()
    }

    pub fn into_parts(self) -> (MatrixConnection, Arc<MatrixSdkHandoffGateway>) {
        (self.matrix, self.handoff)
    }
}

#[derive(Debug, Clone)]
enum MatrixSdkStore {
    Memory,
    EncryptedSqlite(MatrixSdkStoreConfiguration),
}

impl MatrixClientFactory for MatrixSdkClientFactory {
    fn login<'a>(
        &'a self,
        login: &'a MatrixLogin,
    ) -> PortFuture<'a, MatrixResult<MatrixConnection>> {
        Box::pin(async move {
            let client = self.build_client(MatrixOperation::Login).await?;
            let identifier =
                UserIdentifier::Matrix(MatrixUserIdentifier::new(login.login_id().to_owned()));
            let login_info = LoginInfo::Password(Password::new(
                identifier,
                login.password().expose().to_owned(),
            ));
            let mut request = LoginRequest::new(login_info);
            request.device_id = login
                .device_id()
                .map(|device_id| OwnedDeviceId::from(device_id.as_str()));
            request.initial_device_display_name =
                login.initial_device_display_name().map(ToOwned::to_owned);
            request.refresh_token = true;
            let response = client
                .send(request)
                .with_request_config(self.request_config())
                .await
                .map_err(|error| map_http_error(MatrixOperation::Login, &error))?;
            let session = SdkMatrixSession {
                meta: SessionMeta {
                    user_id: response.user_id,
                    device_id: response.device_id,
                },
                tokens: SessionTokens {
                    access_token: response.access_token,
                    refresh_token: response.refresh_token,
                },
            };
            client
                .matrix_auth()
                .restore_session(session, RoomLoadSettings::default())
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::Login, &error))?;
            connection_from_client(
                client,
                MatrixOperation::Login,
                self.configuration.sync_timeline_limit(),
            )
        })
    }

    fn restore<'a>(
        &'a self,
        session: &'a MatrixSession,
    ) -> PortFuture<'a, MatrixResult<MatrixConnection>> {
        Box::pin(async move {
            let client = self.restore_client(session).await?;
            connection_from_client(
                client,
                MatrixOperation::RestoreSession,
                self.configuration.sync_timeline_limit(),
            )
        })
    }
}

#[derive(Debug)]
struct MatrixSdkGateway {
    client: Client,
    metadata: MatrixSessionMetadata,
    sync_timeline_limit: NonZeroU16,
}

impl MatrixGateway for MatrixSdkGateway {
    fn metadata(&self) -> &MatrixSessionMetadata {
        &self.metadata
    }

    fn sync_once<'a>(
        &'a self,
        request: &'a MatrixSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        Box::pin(async move {
            let token = request.since().map_or(SyncToken::NoToken, |value| {
                SyncToken::Specific(value.as_str().to_owned())
            });
            let settings = SyncSettings::new()
                .token(token)
                .timeout(Duration::from_millis(request.timeout().value()))
                .filter(sync_filter(self.sync_timeline_limit))
                .full_state(request.full_state());
            let response = self
                .client
                .sync_once(settings)
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::Sync, &error))?;
            // Matrix SDK 会把 `/keys/upload` 的重复一次性密钥错误吞进后台加密任务，
            // `sync_once` 本身仍可能返回成功。同步边界必须主动读取持久故障标记，
            // 否则 Bridge 会假装在线并永久重试同一组无效密钥。
            reject_recorded_crypto_identity_conflict(&self.client, MatrixOperation::Sync).await?;
            map_sync_response(&response)
        })
    }

    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let mut sdk_request = CreateRoomRequest::new();
            sdk_request.name = request.name().map(ToOwned::to_owned);
            sdk_request.topic = request.topic().map(ToOwned::to_owned);
            sdk_request.visibility = map_visibility(request.visibility());
            sdk_request.preset = Some(map_preset(request.preset()));
            sdk_request.is_direct = request.direct();
            sdk_request.room_alias_name = request
                .alias_localpart()
                .map(|alias| alias.as_str().to_owned());
            sdk_request.creation_content = map_creation_content(request.kind())?;
            if request.encryption() == MatrixRoomEncryption::EndToEnd {
                sdk_request.initial_state.push(
                    InitialStateEvent::with_empty_state_key(
                        RoomEncryptionEventContent::with_recommended_defaults(),
                    )
                    .to_raw_any(),
                );
            }
            sdk_request.invite = request
                .invite()
                .iter()
                .map(|user_id| parse_user_id(user_id, MatrixOperation::CreateRoom))
                .collect::<MatrixResult<Vec<_>>>()?;
            let room = self
                .client
                .create_room(sdk_request)
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::CreateRoom, &error))?;
            MatrixRoomId::new(room.room_id().as_str().to_owned()).map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::CreateRoom,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let user_id =
                parse_user_id(self.metadata.user_id(), MatrixOperation::ResolveRoomAlias)?;
            let alias = RoomAliasId::parse(format!(
                "#{}:{}",
                alias_localpart.as_str(),
                user_id.server_name()
            ))
            .map_err(|_| invalid_response_failure(MatrixOperation::ResolveRoomAlias))?;
            let response = self
                .client
                .resolve_room_alias(&alias)
                .await
                .map_err(|error| map_http_error(MatrixOperation::ResolveRoomAlias, &error))?;
            MatrixRoomId::new(response.room_id.to_string())
                .map_err(|_| invalid_response_failure(MatrixOperation::ResolveRoomAlias))
        })
    }

    fn invite<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let room = self.room(room_id, MatrixOperation::Invite)?;
            let user_id = parse_user_id(user_id, MatrixOperation::Invite)?;
            room.invite_user_by_id(&user_id)
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::Invite, &error))
        })
    }

    fn join<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let room_id = parse_room_id(room_id, MatrixOperation::Join)?;
            self.client
                .join_room_by_id(&room_id)
                .await
                .map(|_| ())
                .map_err(|error| map_sdk_error(MatrixOperation::Join, &error))
        })
    }

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.room(room_id, MatrixOperation::Leave)?
                .leave()
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::Leave, &error))
        })
    }

    fn send_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        Box::pin(async move {
            let room = self.room(room_id, MatrixOperation::SendEvent)?;
            if event_requires_end_to_end_encryption(event) {
                let encrypted = room
                    .latest_encryption_state()
                    .await
                    .map_err(|error| map_sdk_error(MatrixOperation::SendEvent, &error))?
                    .is_encrypted();
                if !encrypted {
                    return Err(MatrixFailure::new(
                        MatrixOperation::SendEvent,
                        MatrixFailureKind::Forbidden,
                    ));
                }
            }
            let transaction_id = OwnedTransactionId::from(event.transaction_id().as_str());
            let response = room
                .send_raw(event.event_type().as_str(), event.content().clone())
                .with_transaction_id(&transaction_id)
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::SendEvent, &error))?;
            let event_id =
                MatrixEventId::new(response.response.event_id.to_string()).map_err(|_| {
                    MatrixFailure::new(
                        MatrixOperation::SendEvent,
                        MatrixFailureKind::InvalidResponse,
                    )
                })?;
            Ok(MatrixAcceptedEvent::new(
                event.transaction_id().clone(),
                event_id,
            ))
        })
    }

    fn send_state_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            let room = self.room(room_id, MatrixOperation::SendStateEvent)?;
            let response = room
                .send_state_event_raw(
                    event.event_type().as_str(),
                    event.state_key().as_str(),
                    event.content().clone(),
                )
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::SendStateEvent, &error))?;
            MatrixEventId::new(response.event_id.to_string()).map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::InvalidResponse,
                )
            })
        })
    }

    fn send_receipt<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        receipt: &'a MatrixReceipt,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let room = self.room(room_id, MatrixOperation::SendReceipt)?;
            let event_id: OwnedEventId = receipt
                .event_id()
                .as_str()
                .try_into()
                .map_err(|_| invalid_response_failure(MatrixOperation::SendReceipt))?;
            room.send_single_receipt(
                map_receipt_type(receipt.kind()),
                ReceiptThread::Unthreaded,
                event_id,
            )
            .await
            .map_err(|error| map_sdk_error(MatrixOperation::SendReceipt, &error))
        })
    }

    fn backfill<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        Box::pin(async move {
            let room = self.room(room_id, MatrixOperation::Backfill)?;
            let mut options = MessagesOptions::backward().from(request.from().as_str());
            options.limit = UInt::from(request.limit().get());
            let response = room
                .messages(options)
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::Backfill, &error))?;
            map_backfill(&response)
        })
    }
}

fn event_requires_end_to_end_encryption(event: &MatrixEvent) -> bool {
    event
        .content()
        .get("content")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|content| content.contains_key("encryption"))
}

impl MatrixRoomAuthorityGateway for MatrixSdkGateway {
    fn inspect_room_authority<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomAuthority>> {
        Box::pin(async move {
            let operation = MatrixOperation::InspectRoomAuthority;
            let room_id = parse_room_id(room_id, operation)?;
            let user_id = parse_user_id(user_id, operation)?;
            let inspector_user_id = parse_user_id(self.metadata.user_id(), operation)?;
            let inspector_is_joined = get_state_content::<RoomMemberEventContent>(
                &self.client,
                room_id.clone(),
                StateEventType::RoomMember,
                inspector_user_id.to_string(),
                operation,
            )
            .await?
            .is_some_and(|content| content.membership == MembershipState::Join);
            if !inspector_is_joined {
                return Err(MatrixFailure::new(operation, MatrixFailureKind::Forbidden));
            }

            let user_is_joined = if inspector_user_id == user_id {
                true
            } else {
                get_state_content::<RoomMemberEventContent>(
                    &self.client,
                    room_id.clone(),
                    StateEventType::RoomMember,
                    user_id.to_string(),
                    operation,
                )
                .await?
                .is_some_and(|content| content.membership == MembershipState::Join)
            };
            if !user_is_joined {
                return Ok(MatrixRoomAuthority::not_joined());
            }

            let (power_levels, create_event) = tokio::try_join!(
                get_state_content::<RoomPowerLevelsEventContent>(
                    &self.client,
                    room_id.clone(),
                    StateEventType::RoomPowerLevels,
                    String::new(),
                    operation,
                ),
                get_room_create_event(&self.client, room_id, operation),
            )?;
            let create_event = create_event.ok_or_else(|| invalid_response_failure(operation))?;
            let power_level = effective_power_level(&user_id, power_levels.as_ref(), &create_event);
            let message_send_power_level =
                effective_message_send_power_level(power_levels.as_ref());
            Ok(MatrixRoomAuthority::joined_with_message_threshold(
                power_level,
                message_send_power_level,
            ))
        })
    }
}

impl MatrixSdkGateway {
    fn room(
        &self,
        room_id: &MatrixRoomId,
        operation: MatrixOperation,
    ) -> MatrixResult<matrix_sdk::Room> {
        let room_id = parse_room_id(room_id, operation)?;
        self.client
            .get_room(&room_id)
            .ok_or_else(|| MatrixFailure::new(operation, MatrixFailureKind::NotFound))
    }
}

fn connection_from_client(
    client: Client,
    operation: MatrixOperation,
    sync_timeline_limit: NonZeroU16,
) -> MatrixResult<MatrixConnection> {
    let (session, sdk_gateway) = sdk_connection_parts(client, operation, sync_timeline_limit)?;
    Ok(application_connection(session, sdk_gateway))
}

fn handoff_connection_from_client(
    client: Client,
    sync_timeline_limit: NonZeroU16,
) -> MatrixResult<MatrixSdkHandoffConnection> {
    let handoff = Arc::new(MatrixSdkHandoffGateway::attach(client.clone()));
    let (session, sdk_gateway) =
        sdk_connection_parts(client, MatrixOperation::RestoreSession, sync_timeline_limit)?;
    Ok(MatrixSdkHandoffConnection {
        matrix: application_connection(session, sdk_gateway),
        handoff,
    })
}

fn sdk_connection_parts(
    client: Client,
    operation: MatrixOperation,
    sync_timeline_limit: NonZeroU16,
) -> MatrixResult<(MatrixSession, Arc<MatrixSdkGateway>)> {
    let sdk_session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| invalid_response_failure(operation))?;
    let session = from_sdk_session(&sdk_session, operation)?;
    let metadata = session.metadata().clone();
    let sdk_gateway = Arc::new(MatrixSdkGateway {
        client,
        metadata,
        sync_timeline_limit,
    });
    Ok((session, sdk_gateway))
}

fn application_connection(
    session: MatrixSession,
    sdk_gateway: Arc<MatrixSdkGateway>,
) -> MatrixConnection {
    let gateway: Arc<dyn MatrixGateway> = sdk_gateway.clone();
    let room_authority_gateway: Arc<dyn MatrixRoomAuthorityGateway> = sdk_gateway;
    MatrixConnection::from_parts(session, gateway, room_authority_gateway)
}

#[derive(Debug, serde::Deserialize)]
struct RoomCreateEventEnvelope {
    sender: String,
    content: RoomCreateEventContent,
}

#[derive(Debug, serde::Deserialize)]
struct RoomCreateEventContent {
    #[serde(default)]
    creator: Option<String>,
    #[serde(default = "default_room_version")]
    room_version: String,
}

async fn get_state_content<T>(
    client: &Client,
    room_id: matrix_sdk::ruma::OwnedRoomId,
    event_type: StateEventType,
    state_key: String,
    operation: MatrixOperation,
) -> MatrixResult<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    let request = GetStateEventRequest::new(room_id, event_type, state_key);
    match client.send(request).await {
        Ok(response) => response
            .into_content()
            .deserialize_as_unchecked::<T>()
            .map(Some)
            .map_err(|_| invalid_response_failure(operation)),
        Err(error) => map_optional_state_error(operation, &error),
    }
}

async fn get_room_create_event(
    client: &Client,
    room_id: matrix_sdk::ruma::OwnedRoomId,
    operation: MatrixOperation,
) -> MatrixResult<Option<RoomCreateEventEnvelope>> {
    let mut request = GetStateEventRequest::new(room_id, StateEventType::RoomCreate, String::new());
    request.format = StateEventFormat::Event;
    match client.send(request).await {
        Ok(response) => serde_json::from_str(response.event_or_content.get())
            .map(Some)
            .map_err(|_| invalid_response_failure(operation)),
        Err(error) => map_optional_state_error(operation, &error),
    }
}

fn map_optional_state_error<T>(
    operation: MatrixOperation,
    error: &matrix_sdk::HttpError,
) -> MatrixResult<Option<T>> {
    let failure = map_http_error(operation, error);
    if failure.kind() == MatrixFailureKind::NotFound {
        Ok(None)
    } else {
        Err(failure)
    }
}

fn effective_power_level(
    user_id: &OwnedUserId,
    power_levels: Option<&RoomPowerLevelsEventContent>,
    create_event: &RoomCreateEventEnvelope,
) -> MatrixPowerLevel {
    let room_version = create_event.content.room_version.parse::<u16>().ok();
    let creator = if room_version.is_some_and(|version| version >= 12) {
        create_event.sender.as_str()
    } else {
        create_event
            .content
            .creator
            .as_deref()
            .unwrap_or(&create_event.sender)
    };
    if creator == user_id.as_str() && room_version.is_some_and(|version| version >= 12) {
        return MatrixPowerLevel::Infinite;
    }
    let finite = power_levels.map_or_else(
        || if creator == user_id.as_str() { 100 } else { 0 },
        |content| {
            i64::from(
                content
                    .users
                    .get(user_id)
                    .copied()
                    .unwrap_or(content.users_default),
            )
        },
    );
    MatrixPowerLevel::finite(finite)
}

fn effective_message_send_power_level(
    power_levels: Option<&RoomPowerLevelsEventContent>,
) -> MatrixPowerLevel {
    let finite = power_levels.map_or(0, |content| {
        i64::from(
            content
                .events
                .get(&TimelineEventType::RoomMessage)
                .copied()
                .unwrap_or(content.events_default),
        )
    });
    MatrixPowerLevel::finite(finite)
}

fn default_room_version() -> String {
    "1".to_owned()
}

fn sync_filter(timeline_limit: NonZeroU16) -> SyncFilter {
    let mut definition = FilterDefinition::empty();
    definition.room.include_leave = true;
    definition.room.timeline.limit = Some(UInt::from(timeline_limit.get()));
    SyncFilter::from(definition)
}

fn from_sdk_session(
    session: &SdkMatrixSession,
    operation: MatrixOperation,
) -> MatrixResult<MatrixSession> {
    let user_id = MatrixUserId::new(session.meta.user_id.to_string())
        .map_err(|_| invalid_response_failure(operation))?;
    let device_id = MatrixDeviceId::new(session.meta.device_id.to_string())
        .map_err(|_| invalid_response_failure(operation))?;
    let access_token = SecretValue::new(session.tokens.access_token.clone())
        .map_err(|_| invalid_response_failure(operation))?;
    let refresh_token = session
        .tokens
        .refresh_token
        .as_ref()
        .map(|value| SecretValue::new(value.clone()))
        .transpose()
        .map_err(|_| invalid_response_failure(operation))?;
    Ok(MatrixSession::new(
        MatrixSessionMetadata::new(user_id, device_id),
        access_token,
        refresh_token,
    ))
}

fn to_sdk_session(session: &MatrixSession) -> MatrixResult<SdkMatrixSession> {
    let user_id: OwnedUserId = session
        .metadata()
        .user_id()
        .as_str()
        .try_into()
        .map_err(|_| invalid_response_failure(MatrixOperation::RestoreSession))?;
    let device_id = OwnedDeviceId::from(session.metadata().device_id().as_str());
    Ok(SdkMatrixSession {
        meta: SessionMeta { user_id, device_id },
        tokens: SessionTokens {
            access_token: session.access_token().expose().to_owned(),
            refresh_token: session
                .refresh_token()
                .map(|value| value.expose().to_owned()),
        },
    })
}

fn parse_room_id(
    room_id: &MatrixRoomId,
    operation: MatrixOperation,
) -> MatrixResult<matrix_sdk::ruma::OwnedRoomId> {
    RoomId::parse(room_id.as_str()).map_err(|_| invalid_response_failure(operation))
}

fn parse_user_id(user_id: &MatrixUserId, operation: MatrixOperation) -> MatrixResult<OwnedUserId> {
    UserId::parse(user_id.as_str()).map_err(|_| invalid_response_failure(operation))
}

const fn map_visibility(value: MatrixRoomVisibility) -> Visibility {
    match value {
        MatrixRoomVisibility::Private => Visibility::Private,
        MatrixRoomVisibility::Public => Visibility::Public,
    }
}

const fn map_preset(value: MatrixRoomPreset) -> RoomPreset {
    match value {
        MatrixRoomPreset::PrivateChat => RoomPreset::PrivateChat,
        MatrixRoomPreset::PublicChat => RoomPreset::PublicChat,
        MatrixRoomPreset::TrustedPrivateChat => RoomPreset::TrustedPrivateChat,
    }
}

fn map_creation_content(kind: MatrixRoomKind) -> MatrixResult<Option<Raw<CreationContent>>> {
    if kind == MatrixRoomKind::Conversation {
        return Ok(None);
    }
    let mut content = CreationContent::new();
    content.room_type = Some(RoomType::Space);
    Raw::new(&content)
        .map(Some)
        .map_err(|_| invalid_response_failure(MatrixOperation::CreateRoom))
}

const fn map_receipt_type(value: MatrixReceiptKind) -> ReceiptType {
    match value {
        MatrixReceiptKind::Read => ReceiptType::Read,
        MatrixReceiptKind::PrivateRead => ReceiptType::ReadPrivate,
        MatrixReceiptKind::FullyRead => ReceiptType::FullyRead,
    }
}

const fn invalid_response_failure(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use agent_room_application::ports::{
        MatrixFailureKind, MatrixOperation, MatrixRoomKind, SecretValue,
    };
    use matrix_sdk::{Client, ruma::room::RoomType};
    use matrix_sdk_base::store::{StateStoreDataKey, StateStoreDataValue};
    use tempfile::tempdir;

    use crate::{
        MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkStoreConfiguration,
        sdk::{map_creation_content, reject_recorded_crypto_identity_conflict},
    };

    #[test]
    fn space_创建内容使用标准_m_space_房间类型() {
        assert!(
            map_creation_content(MatrixRoomKind::Conversation)
                .expect("普通房间映射成功")
                .is_none()
        );
        let raw = map_creation_content(MatrixRoomKind::Space)
            .expect("Space 映射成功")
            .expect("Space 必须携带创建内容");
        let content = raw.deserialize().expect("Space 创建内容可反序列化");
        assert_eq!(content.room_type, Some(RoomType::Space));
    }

    #[tokio::test]
    async fn 加密_sqlite_store_可持久化且无需联网初始化() {
        let directory = tempdir().expect("临时目录可创建");
        let store_path = directory.path().join("matrix-store");
        let factory =
            persistent_factory(store_path.clone(), "matrix-store-passphrase-000000000001");

        factory
            .initialize_store()
            .await
            .expect("Store 应成功初始化");

        let entries = fs::read_dir(store_path)
            .expect("Store 目录应存在")
            .collect::<Result<Vec<_>, _>>()
            .expect("Store 目录应可读取");
        assert!(!entries.is_empty(), "Matrix SDK 必须创建持久文件");
    }

    #[tokio::test]
    async fn 错误口令不能打开既有_store() {
        let directory = tempdir().expect("临时目录可创建");
        let store_path = directory.path().join("matrix-store");
        persistent_factory(store_path.clone(), "matrix-store-passphrase-000000000001")
            .initialize_store()
            .await
            .expect("首个口令应建立 Store");

        let failure = persistent_factory(store_path, "matrix-store-passphrase-000000000002")
            .initialize_store()
            .await
            .expect_err("错误口令必须被拒绝");

        assert_eq!(failure.operation(), MatrixOperation::InitializeStore);
        assert_eq!(failure.kind(), MatrixFailureKind::DependencyUnavailable);
    }

    #[tokio::test]
    async fn 已持久化的重复一次性密钥标记会拒绝恢复旧设备身份() {
        let client = Client::builder()
            .homeserver_url("http://127.0.0.1:18008")
            .build()
            .await
            .expect("内存客户端可创建");
        client
            .state_store()
            .set_kv_data(
                StateStoreDataKey::OneTimeKeyAlreadyUploaded,
                StateStoreDataValue::OneTimeKeyAlreadyUploaded,
            )
            .await
            .expect("重复密钥标记可持久化");

        let failure =
            reject_recorded_crypto_identity_conflict(&client, MatrixOperation::RestoreSession)
                .await
                .expect_err("旧设备加密身份必须触发上层的一次性恢复");

        assert_eq!(failure.operation(), MatrixOperation::RestoreSession);
        assert_eq!(
            failure.kind(),
            MatrixFailureKind::CryptographicIdentityConflict
        );
    }

    #[tokio::test]
    async fn 同步边界读取重复一次性密钥标记时保留同步操作维度() {
        let client = Client::builder()
            .homeserver_url("http://127.0.0.1:18008")
            .build()
            .await
            .expect("内存客户端可创建");
        client
            .state_store()
            .set_kv_data(
                StateStoreDataKey::OneTimeKeyAlreadyUploaded,
                StateStoreDataValue::OneTimeKeyAlreadyUploaded,
            )
            .await
            .expect("重复密钥标记可持久化");

        let failure = reject_recorded_crypto_identity_conflict(&client, MatrixOperation::Sync)
            .await
            .expect_err("同步边界必须把后台加密冲突提升到 Bridge");

        assert_eq!(failure.operation(), MatrixOperation::Sync);
        assert_eq!(
            failure.kind(),
            MatrixFailureKind::CryptographicIdentityConflict
        );
    }

    #[test]
    fn 调试输出不会泄露_store_口令() {
        let directory = tempdir().expect("临时目录可创建");
        let passphrase = "matrix-store-passphrase-000000000001";
        let factory = persistent_factory(directory.path().join("matrix-store"), passphrase);

        let debug = format!("{factory:?}");
        assert!(!debug.contains(passphrase));
        assert!(debug.contains("[已脱敏]"));
    }

    fn persistent_factory(
        store_path: std::path::PathBuf,
        passphrase: &str,
    ) -> MatrixSdkClientFactory {
        let sdk = MatrixSdkConfiguration::new("http://127.0.0.1:18008", Duration::from_secs(5))
            .expect("SDK 配置有效");
        let store = MatrixSdkStoreConfiguration::encrypted_sqlite(
            store_path,
            SecretValue::new(passphrase).expect("Store 口令有效"),
        )
        .expect("Store 配置有效");
        MatrixSdkClientFactory::with_encrypted_sqlite(sdk, store)
    }
}
