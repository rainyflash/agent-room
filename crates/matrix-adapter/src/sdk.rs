use std::{num::NonZeroU16, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixBackfillPage, MatrixBackfillRequest, MatrixClientFactory,
    MatrixConnection, MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId, MatrixFailure,
    MatrixFailureKind, MatrixGateway, MatrixLogin, MatrixOperation, MatrixReceipt,
    MatrixReceiptKind, MatrixResult, MatrixRoomId, MatrixRoomPreset, MatrixRoomVisibility,
    MatrixSession, MatrixSessionMetadata, MatrixStateEvent, MatrixSyncBatch, MatrixSyncRequest,
    MatrixUserId, PortFuture, SecretValue,
};
use matrix_sdk::{
    Client, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession as SdkMatrixSession,
    config::{RequestConfig, SyncSettings, SyncToken},
    room::MessagesOptions,
    ruma::{
        OwnedDeviceId, OwnedEventId, OwnedTransactionId, OwnedUserId, RoomId, UInt, UserId,
        api::client::{
            filter::FilterDefinition,
            receipt::create_receipt::v3::ReceiptType,
            room::{
                Visibility,
                create_room::v3::{Request as CreateRoomRequest, RoomPreset},
            },
            session::login::v3::{LoginInfo, Password, Request as LoginRequest},
            sync::sync_events::v3::Filter as SyncFilter,
            uiaa::{MatrixUserIdentifier, UserIdentifier},
        },
        events::receipt::ReceiptThread,
    },
    store::RoomLoadSettings,
};

use crate::{
    configuration::{MatrixSdkConfiguration, MatrixSdkStoreConfiguration},
    error::{map_build_error, map_http_error, map_sdk_error},
    mapping::{map_backfill, map_sync_response},
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
        drop(self.build_client(MatrixOperation::InitializeStore).await?);
        Ok(())
    }

    async fn build_client(&self, operation: MatrixOperation) -> MatrixResult<Client> {
        let builder = Client::builder()
            .homeserver_url(self.configuration.homeserver_url().clone())
            .request_config(self.request_config());
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

    fn request_config(&self) -> RequestConfig {
        RequestConfig::new()
            .disable_retry()
            .timeout(self.configuration.request_timeout())
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
            let client = self.build_client(MatrixOperation::RestoreSession).await?;
            let sdk_session = to_sdk_session(session)?;
            client
                .matrix_auth()
                .restore_session(sdk_session, RoomLoadSettings::default())
                .await
                .map_err(|error| map_sdk_error(MatrixOperation::RestoreSession, &error))?;
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
    let sdk_session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| invalid_response_failure(operation))?;
    let session = from_sdk_session(&sdk_session, operation)?;
    let metadata = session.metadata().clone();
    let gateway: Arc<dyn MatrixGateway> = Arc::new(MatrixSdkGateway {
        client,
        metadata,
        sync_timeline_limit,
    });
    Ok(MatrixConnection::from_parts(session, gateway))
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

    use agent_room_application::ports::{MatrixFailureKind, MatrixOperation, SecretValue};
    use tempfile::tempdir;

    use crate::{MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkStoreConfiguration};

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
