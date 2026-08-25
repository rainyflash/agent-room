use std::{sync::Arc, time::Duration};

use agent_room_application::{
    authentication::AuthenticationUseCases,
    content::{
        BeginContentUploadDependencies, BeginContentUploadService, BindContentEventDependencies,
        BindContentEventService, CleanupContentDependencies, CleanupContentPolicy,
        CleanupContentService, CompleteContentUploadDependencies, CompleteContentUploadService,
        ContentCleanupUseCases, ContentMembershipAuthorizationDependencies,
        ContentMembershipAuthorizationService, ContentReadTicketLifetime, ContentService,
        ContentServiceDependencies, ContentUseCases, IssueContentReadTicketDependencies,
        IssueContentReadTicketService, OpenContentDependencies, OpenContentService,
        RedactContentDependencies, RedactContentService,
    },
    devices::DeviceAuthorizationUseCases,
    ports::{
        ContentDownloadLimiter, ContentMembershipAuthorizer, ContentPrincipalIdentityLookup,
        ContentReadTicketCodec, ContentRepository, ContentScanner, ContentStorageKeyFactory,
        MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
        MatrixAgentUserRegistration, MatrixClientFactory, MatrixDeviceId, MatrixFailure,
        MatrixFailureKind, MatrixOperation, MatrixRoomAuthority, MatrixRoomAuthorityGateway,
        MatrixRoomId, MatrixUserId, PortFuture, PrivateContentObjectStore, RoomMembershipGateway,
        SecretFactory, SecretValue,
    },
};
use agent_room_content_adapter::{
    ClamAvContentScanner, ClamAvScannerConfig, ContentTicketSigningKey, HmacContentReadTicketCodec,
    S3ContentStoreConfig, S3PrivateContentObjectStore, SecureContentStorageKeyFactory,
};
use agent_room_domain::{rooms::MatrixRoomReference, time::DurationMillis};
use agent_room_matrix_adapter::{MatrixSdkClientFactory, MatrixSdkConfiguration};
use agent_room_matrix_provisioning_adapter::MatrixApplicationServiceProvisioner;
use agent_room_postgres_adapter::{
    ContentDownloadLimitPolicy, PostgresContentDownloadLimiter, PostgresRepositories,
};
use axum::Router;

use crate::{
    StartupError,
    config::ContentConfig,
    content_cleanup::ContentCleanupWorker,
    features::content::{ContentHttpDependencies, ContentHttpState},
    runtime::SystemRuntime,
};

const AUTHORITY_DEVICE_ID: &str = "AR_CONTENT_AUTHORITY";
const AUTHORITY_DEVICE_NAME: &str = "Agent Room 内容授权服务";

pub(crate) struct ContentRuntime {
    routes: Router,
    cleanup: ContentCleanupWorker,
}

impl ContentRuntime {
    pub(crate) fn into_parts(self) -> (Router, ContentCleanupWorker) {
        (self.routes, self.cleanup)
    }
}

pub(crate) struct ContentRuntimeDependencies<'a> {
    pub(crate) config: &'a ContentConfig,
    pub(crate) matrix_base_url: &'a str,
    pub(crate) matrix_request_timeout: Duration,
    pub(crate) repositories: Arc<PostgresRepositories>,
    pub(crate) system_runtime: Arc<SystemRuntime>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
    pub(crate) matrix_identities: Arc<MatrixApplicationServiceProvisioner>,
    pub(crate) frontend_origin: &'a url::Url,
}

struct ContentApplication {
    use_cases: Arc<dyn ContentUseCases>,
    cleanup: Arc<dyn ContentCleanupUseCases>,
}

struct ContentApplicationDependencies<'a> {
    config: &'a ContentConfig,
    repository: Arc<dyn ContentRepository>,
    system_runtime: Arc<SystemRuntime>,
    object_store: Arc<dyn PrivateContentObjectStore>,
    scanner: Arc<dyn ContentScanner>,
    ticket_codec: Arc<dyn ContentReadTicketCodec>,
    authorizer: Arc<dyn ContentMembershipAuthorizer>,
    limiter: Arc<dyn ContentDownloadLimiter>,
}

/// 装配内容服务的全部外部适配器，并建立权威 Matrix 读取会话。
///
/// # Errors
///
/// 任一基础设施配置非法、Matrix 服务身份无法签发或恢复时返回脱敏启动错误。
pub(crate) async fn initialize(
    dependencies: ContentRuntimeDependencies<'_>,
) -> Result<ContentRuntime, StartupError> {
    let object_store = build_object_store(dependencies.config)?;
    let scanner = build_scanner(dependencies.config, object_store.clone())?;
    let ticket_codec = build_ticket_codec(dependencies.config)?;
    let authority = build_matrix_authority(
        dependencies.config,
        dependencies.matrix_base_url,
        dependencies.matrix_request_timeout,
        dependencies.matrix_identities,
    )
    .await?;
    let repository: Arc<dyn ContentRepository> = dependencies.repositories.clone();
    let identities: Arc<dyn ContentPrincipalIdentityLookup> = dependencies.repositories.clone();
    let authorizer: Arc<dyn ContentMembershipAuthorizer> = Arc::new(
        ContentMembershipAuthorizationService::new(ContentMembershipAuthorizationDependencies {
            identities,
            matrix_authority: authority,
            private_rooms: dependencies.repositories.clone(),
        }),
    );
    let limiter = build_download_limiter(dependencies.config, &dependencies.repositories)?;
    let application = build_content_application(ContentApplicationDependencies {
        config: dependencies.config,
        repository,
        system_runtime: dependencies.system_runtime,
        object_store,
        scanner,
        ticket_codec,
        authorizer,
        limiter,
    })?;
    let cleanup =
        ContentCleanupWorker::start(application.cleanup, dependencies.config.cleanup_interval)
            .map_err(|error| {
                StartupError::new(
                    "startup.invalid_content_cleanup_schedule",
                    error.to_string(),
                )
            })?;
    let state = ContentHttpState::new(
        ContentHttpDependencies {
            content: application.use_cases,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
        },
        dependencies.frontend_origin,
    );
    Ok(ContentRuntime {
        routes: crate::features::content::router(state),
        cleanup,
    })
}

fn build_content_application(
    dependencies: ContentApplicationDependencies<'_>,
) -> Result<ContentApplication, StartupError> {
    let begin_upload = Arc::new(BeginContentUploadService::new(
        BeginContentUploadDependencies {
            clock: dependencies.system_runtime.clone(),
            identifiers: dependencies.system_runtime.clone(),
            storage_keys: Arc::new(SecureContentStorageKeyFactory)
                as Arc<dyn ContentStorageKeyFactory>,
            repository: dependencies.repository.clone(),
            authorizer: dependencies.authorizer.clone(),
        },
    ));
    let complete_upload = Arc::new(CompleteContentUploadService::new(
        CompleteContentUploadDependencies {
            clock: dependencies.system_runtime.clone(),
            repository: dependencies.repository.clone(),
            object_store: dependencies.object_store.clone(),
            scanner: dependencies.scanner,
        },
    ));
    let bind_event = Arc::new(BindContentEventService::new(BindContentEventDependencies {
        clock: dependencies.system_runtime.clone(),
        repository: dependencies.repository.clone(),
    }));
    let redact = Arc::new(RedactContentService::new(RedactContentDependencies {
        clock: dependencies.system_runtime.clone(),
        repository: dependencies.repository.clone(),
    }));
    let issue_read_ticket = Arc::new(IssueContentReadTicketService::new(
        IssueContentReadTicketDependencies {
            clock: dependencies.system_runtime.clone(),
            repository: dependencies.repository.clone(),
            authorizer: dependencies.authorizer.clone(),
            ticket_codec: dependencies.ticket_codec.clone(),
            lifetime: build_read_ticket_lifetime(dependencies.config)?,
        },
    ));
    let open = Arc::new(OpenContentService::new(OpenContentDependencies {
        clock: dependencies.system_runtime.clone(),
        repository: dependencies.repository.clone(),
        authorizer: dependencies.authorizer,
        ticket_codec: dependencies.ticket_codec,
        limiter: dependencies.limiter,
        object_store: dependencies.object_store.clone(),
    }));
    let use_cases = Arc::new(ContentService::new(ContentServiceDependencies {
        begin_upload,
        complete_upload,
        bind_event,
        redact,
        issue_read_ticket,
        open,
    })) as Arc<dyn ContentUseCases>;
    let cleanup = Arc::new(CleanupContentService::new(CleanupContentDependencies {
        clock: dependencies.system_runtime,
        repository: dependencies.repository,
        object_store: dependencies.object_store,
        policy: build_cleanup_policy(dependencies.config)?,
    })) as Arc<dyn ContentCleanupUseCases>;
    Ok(ContentApplication { use_cases, cleanup })
}

fn build_read_ticket_lifetime(
    config: &ContentConfig,
) -> Result<ContentReadTicketLifetime, StartupError> {
    ContentReadTicketLifetime::new(duration_millis(
        config.read_ticket_ttl,
        "startup.invalid_content_ticket_lifetime",
    )?)
    .map_err(|error| {
        StartupError::new("startup.invalid_content_ticket_lifetime", error.to_string())
    })
}

fn build_cleanup_policy(config: &ContentConfig) -> Result<CleanupContentPolicy, StartupError> {
    CleanupContentPolicy::new(
        duration_millis(
            config.orphan_grace,
            "startup.invalid_content_cleanup_policy",
        )?,
        config.cleanup_batch,
    )
    .map_err(|error| StartupError::new("startup.invalid_content_cleanup_policy", error.to_string()))
}

fn build_object_store(
    config: &ContentConfig,
) -> Result<Arc<dyn PrivateContentObjectStore>, StartupError> {
    let configuration = S3ContentStoreConfig::new(
        config.object_store_endpoint.as_str(),
        config.object_store_bucket.clone(),
        config.object_store_region.clone(),
        application_secret(&config.object_store_access_key)?,
        application_secret(&config.object_store_secret_key)?,
        config.object_store_timeout,
    )
    .map_err(|error| StartupError::new("startup.invalid_content_store", error.to_string()))?;
    Ok(Arc::new(S3PrivateContentObjectStore::new(&configuration)))
}

fn build_scanner(
    config: &ContentConfig,
    object_store: Arc<dyn PrivateContentObjectStore>,
) -> Result<Arc<dyn ContentScanner>, StartupError> {
    let configuration = ClamAvScannerConfig::new(
        config.scanner_address,
        config.scanner_connect_timeout,
        config.scanner_timeout,
    )
    .map_err(|error| StartupError::new("startup.invalid_content_scanner", error.to_string()))?;
    Ok(Arc::new(ClamAvContentScanner::new(
        configuration,
        object_store,
    )))
}

fn build_ticket_codec(
    config: &ContentConfig,
) -> Result<Arc<dyn ContentReadTicketCodec>, StartupError> {
    let active = ContentTicketSigningKey::new(
        config.ticket_key_id.clone(),
        application_secret(&config.ticket_secret)?,
    )
    .map_err(|error| StartupError::new("startup.invalid_content_ticket_key", error.to_string()))?;
    let codec = HmacContentReadTicketCodec::new(active, Vec::new()).map_err(|error| {
        StartupError::new("startup.invalid_content_ticket_key", error.to_string())
    })?;
    Ok(Arc::new(codec))
}

fn build_download_limiter(
    config: &ContentConfig,
    repositories: &PostgresRepositories,
) -> Result<Arc<dyn ContentDownloadLimiter>, StartupError> {
    let window = DurationMillis::new(duration_millis(
        config.download_window,
        "startup.invalid_content_download_limit",
    )?)
    .map_err(|error| {
        StartupError::new("startup.invalid_content_download_limit", error.to_string())
    })?;
    let policy = ContentDownloadLimitPolicy::new(
        window,
        config.download_max_requests,
        config.download_max_bytes,
    )
    .map_err(|error| {
        StartupError::new("startup.invalid_content_download_limit", error.to_string())
    })?;
    Ok(Arc::new(PostgresContentDownloadLimiter::new(
        repositories.pool().clone(),
        policy,
    )))
}

async fn build_matrix_authority(
    config: &ContentConfig,
    matrix_base_url: &str,
    request_timeout: Duration,
    identities: Arc<MatrixApplicationServiceProvisioner>,
) -> Result<Arc<dyn MatrixRoomAuthorityGateway>, StartupError> {
    let registration = MatrixAgentUserRegistration::new(MatrixAgentLocalpart::from_agent_id(
        config.matrix_authority_agent_id,
    ));
    let user_id = identities
        .ensure_user(&registration)
        .await
        .map_err(|error| matrix_startup_failure("startup.content_matrix_identity_failed", error))?;
    let device_id = MatrixDeviceId::new(AUTHORITY_DEVICE_ID).map_err(|_| {
        StartupError::new(
            "startup.invalid_content_matrix_identity",
            "内容授权设备标识无效".to_owned(),
        )
    })?;
    let membership = identities
        .room_membership(user_id.clone())
        .map_err(|error| {
            matrix_startup_failure("startup.content_matrix_membership_failed", error)
        })?;
    let session_request =
        MatrixAgentDeviceSessionRequest::new(user_id, device_id, AUTHORITY_DEVICE_NAME.to_owned())
            .map_err(|error| {
                StartupError::new("startup.invalid_content_matrix_identity", error.to_string())
            })?;
    let session = identities
        .issue_device_session(&session_request)
        .await
        .map_err(|error| matrix_startup_failure("startup.content_matrix_session_failed", error))?;
    let configuration =
        MatrixSdkConfiguration::new(matrix_base_url, request_timeout).map_err(|error| {
            StartupError::new("startup.invalid_content_matrix_client", error.to_string())
        })?;
    let connection = MatrixSdkClientFactory::new(configuration)
        .restore(&session)
        .await
        .map_err(|error| matrix_startup_failure("startup.content_matrix_restore_failed", error))?;
    Ok(Arc::new(MembershipEnsuringRoomAuthority {
        membership: Arc::new(membership),
        authority: connection.room_authority_gateway_handle(),
    }))
}

/// 内容授权服务用独立 Matrix 身份读取权威状态；每次检查前先幂等入房。
///
/// 公共房间可直接加入，私有房间则必须由建房策略预先邀请该服务身份。
struct MembershipEnsuringRoomAuthority {
    membership: Arc<dyn RoomMembershipGateway>,
    authority: Arc<dyn MatrixRoomAuthorityGateway>,
}

impl MatrixRoomAuthorityGateway for MembershipEnsuringRoomAuthority {
    fn inspect_room_authority<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, Result<MatrixRoomAuthority, MatrixFailure>> {
        Box::pin(async move {
            let room = MatrixRoomReference::new(room_id.as_str().to_owned()).map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::InspectRoomAuthority,
                    MatrixFailureKind::InvalidResponse,
                )
            })?;
            self.membership.join(&room).await?;
            self.authority
                .inspect_room_authority(room_id, user_id)
                .await
        })
    }
}

fn application_secret(value: &crate::config::SecretValue) -> Result<SecretValue, StartupError> {
    SecretValue::new(value.expose().to_owned()).map_err(|_| {
        StartupError::new(
            "startup.invalid_content_secret",
            "内容服务密钥格式无效".to_owned(),
        )
    })
}

fn matrix_startup_failure(code: &'static str, failure: MatrixFailure) -> StartupError {
    StartupError::new(
        code,
        format!(
            "Matrix 操作失败：{:?}/{:?}",
            failure.operation(),
            failure.kind()
        ),
    )
}

fn duration_millis(duration: Duration, code: &'static str) -> Result<u64, StartupError> {
    u64::try_from(duration.as_millis())
        .map_err(|_| StartupError::new(code, "时长超出支持范围".to_owned()))
}
