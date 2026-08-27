mod account_deletion;
mod config;
mod content_cleanup;
mod content_runtime;
mod correlation;
mod error;
mod features;
mod observability;
mod operational_metrics;
mod runtime;
mod shutdown;
mod telemetry_metrics;

use std::{error::Error, fmt, sync::Arc, time::Duration};

use agent_room_a2a_adapter::{
    AgentCardNormalizer, HttpsDocumentClient, PinnedHttpsClient, PinnedHttpsClientConfiguration,
    RemoteAgentCardSource, SystemDnsResolver,
};
use agent_room_application::{
    account_lifecycle::{
        AccountDeletionWorker, AccountDeletionWorkerDependencies, AccountLifecycleDependencies,
        AccountLifecycleService,
    },
    agent_cards::{AgentCardDependencies, AgentCardService},
    agent_instance_management::{
        AgentInstanceManagementDependencies, AgentInstanceManagementService,
    },
    agent_instance_verification::{
        AgentInstanceVerificationDependencies, AgentInstanceVerificationService,
    },
    agent_lobbies::{AgentLobbyEntryDependencies, AgentLobbyEntryService},
    agents::{AgentManagementDependencies, AgentManagementService},
    authentication::{AuthenticationDependencies, AuthenticationPolicy, AuthenticationService},
    automation::{AutomationDependencies, AutomationService},
    devices::{
        DeviceAuthorizationDependencies, DeviceAuthorizationPolicy, DeviceAuthorizationService,
    },
    direct_sessions::{DirectSessionDependencies, DirectSessionService},
    handoffs::{HandoffAccessDependencies, HandoffAccessService},
    health::ReadinessService,
    lobby_observation::PublicLobbyObservationService,
    moderation::{ModerationDependencies, ModerationService},
    ports::{MatrixAgentLocalpart, MatrixRoomAuthorityGateway, MatrixUserId, SecretValue},
    private_rooms::{PrivateRoomDependencies, PrivateRoomService},
    rooms::{
        LobbyJoinPolicy, LobbyProvisioningDependencies, LobbyProvisioningPolicy,
        LobbyProvisioningService,
    },
};
use agent_room_domain::time::DurationMillis;
use agent_room_identity_adapter::{
    DiscoveredOidcDeviceGrant, DiscoveredOidcGateway, Ed25519DeviceProofVerifier,
    HmacAccountDeletionReceiptIssuer, OidcAdapterConfig, OidcDeviceGrantConfig,
    SecureSecretFactory,
};
use agent_room_matrix_provisioning_adapter::{
    MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
    SynapseAccountLifecycleConfiguration, SynapseAccountLifecycleGateway,
};
use agent_room_postgres_adapter::PostgresRepositories;
use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
    routing::get,
};
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, CorsLayer};

use config::{AccountLifecycleConfig, AuthenticationConfig, ControlPlaneConfig, LobbyConfig};
use features::accounts::AccountHttpState;
use features::agent_cards::{AgentCardHttpDependencies, AgentCardHttpState};
use features::agent_instances::{AgentInstanceHttpState, AgentInstanceHttpStateDependencies};
use features::agents::{AgentHttpDependencies, AgentHttpState};
use features::authentication::AuthenticationHttpState;
use features::automation::{AutomationHttpDependencies, AutomationHttpState};
use features::devices::{DeviceHttpDependencies, DeviceHttpState};
use features::direct_sessions::DirectSessionHttpState;
use features::handoffs::{HandoffHttpDependencies, HandoffHttpState};
use features::health::HealthRuntime;
use features::lobbies::{LobbyHttpDependencies, LobbyHttpState};
use features::moderation::ModerationHttpState;
use features::private_rooms::PrivateRoomHttpState;
use features::telemetry::FrontendTelemetryHttpState;
use observability::Observability;
use runtime::SystemRuntime;
use telemetry_metrics::TelemetryMetrics;

pub(crate) const SERVICE_NAME: &str = "agent-room-control-plane";
pub(crate) const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub(crate) struct AppState {
    readiness: Arc<ReadinessService>,
    metrics: TelemetryMetrics,
}

struct IdentityRuntime {
    routes: Router,
    content_cleanup: content_cleanup::ContentCleanupWorker,
    account_deletion: account_deletion::AccountDeletionRuntime,
    operational_metrics: operational_metrics::OperationalMetricsRuntime,
}

struct AgentFeatureHttpStates {
    agents: AgentHttpState,
    instances: AgentInstanceHttpState,
    cards: AgentCardHttpState,
    handoffs: HandoffHttpState,
    lobbies: LobbyHttpState,
    private_rooms: PrivateRoomHttpState,
    direct_sessions: DirectSessionHttpState,
    automation: AutomationHttpState,
    moderation: ModerationHttpState,
}

struct AgentFeatureDependencies {
    repositories: Arc<PostgresRepositories>,
    system_runtime: Arc<SystemRuntime>,
    secrets: Arc<SecureSecretFactory>,
    matrix_identities: Arc<MatrixApplicationServiceProvisioner>,
    authentication: Arc<AuthenticationService>,
    devices: Arc<DeviceAuthorizationService>,
    matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
}

/// 从进程环境启动控制平面，并在终止信号后释放数据库与遥测资源。
///
/// # Errors
///
/// 配置、监听端口、依赖初始化、遥测初始化或 HTTP 服务失败时返回脱敏启动错误。
pub async fn run() -> Result<(), StartupError> {
    let config = ControlPlaneConfig::from_environment()
        .map_err(|error| StartupError::new("startup.invalid_config", error.to_string()))?;
    let listener = TcpListener::bind(config.bind_address)
        .await
        .map_err(|error| {
            StartupError::new(
                "startup.bind_failed",
                format!("监听地址绑定失败：{:?}", error.kind()),
            )
        })?;
    let observability = Observability::install(&config.observability)
        .map_err(|error| StartupError::new("startup.telemetry_failed", error.to_string()))?;
    let runtime = match HealthRuntime::initialize(&config.dependencies) {
        Ok(runtime) => runtime,
        Err(error) => {
            observability.shutdown();
            return Err(StartupError::new(
                "startup.dependencies_failed",
                error.to_string(),
            ));
        }
    };
    let metrics = observability.metrics();
    let identity_runtime = match build_identity_router(&config, &runtime, metrics.clone()).await {
        Ok(runtime) => runtime,
        Err(error) => {
            runtime.shutdown().await;
            observability.shutdown();
            return Err(error);
        }
    };
    let IdentityRuntime {
        routes,
        content_cleanup,
        account_deletion,
        operational_metrics,
    } = identity_runtime;
    let app = build_router(
        runtime.readiness.clone(),
        routes,
        &config.authentication.frontend_origin,
        metrics,
    )?;

    tracing::info!(
        service = SERVICE_NAME,
        version = SERVICE_VERSION,
        bind_address = %config.bind_address,
        "控制平面已启动"
    );
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown::signal())
        .await;

    content_cleanup.shutdown().await;
    account_deletion.shutdown().await;
    operational_metrics.shutdown().await;
    runtime.shutdown().await;
    observability.shutdown();
    result.map_err(|error| {
        StartupError::new(
            "runtime.server_failed",
            format!("HTTP 服务异常结束：{:?}", error.kind()),
        )
    })
}

fn build_router(
    readiness: Arc<ReadinessService>,
    feature_routes: Router,
    frontend_origin: &url::Url,
    metrics: TelemetryMetrics,
) -> Result<Router, StartupError> {
    let origin =
        HeaderValue::from_str(&frontend_origin.origin().ascii_serialization()).map_err(|_| {
            StartupError::new(
                "startup.invalid_authentication_config",
                "前端 Origin 无法转换为安全 HTTP 头".to_owned(),
            )
        })?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact(origin))
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("idempotency-key"),
            HeaderName::from_static("x-agent-room-content-byte-length"),
            HeaderName::from_static("x-agent-room-content-sha256"),
            HeaderName::from_static("x-agent-room-device-id"),
            HeaderName::from_static("x-agent-room-proof-issued-at"),
            HeaderName::from_static("x-agent-room-proof-nonce"),
            HeaderName::from_static("x-agent-room-proof-signature"),
        ])
        .expose_headers([
            header::CACHE_CONTROL,
            header::RETRY_AFTER,
            HeaderName::from_static("content-digest"),
            HeaderName::from_static(correlation::CORRELATION_ID_HEADER),
        ]);

    Ok(Router::new()
        .route("/health/live", get(features::health::live))
        .route("/health/ready", get(features::health::ready))
        .route("/capabilities", get(features::capabilities::get))
        .fallback(error::not_found)
        .method_not_allowed_fallback(error::method_not_allowed)
        .with_state(AppState {
            readiness,
            metrics: metrics.clone(),
        })
        .merge(feature_routes)
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            metrics,
            telemetry_metrics::record_http_request,
        ))
        .layer(middleware::from_fn(correlation::attach)))
}

async fn build_identity_router(
    config: &ControlPlaneConfig,
    runtime: &HealthRuntime,
    metrics: TelemetryMetrics,
) -> Result<IdentityRuntime, StartupError> {
    let authentication_config = &config.authentication;
    let request_timeout = config.dependencies.timeout;
    let oidc = build_web_oidc(authentication_config, request_timeout)?;
    let device_oidc = build_device_oidc(authentication_config, request_timeout)?;
    let policy = authentication_policy(authentication_config)?;
    let repositories = Arc::new(PostgresRepositories::new(runtime.pool().clone()));
    let system_runtime = Arc::new(SystemRuntime);
    let secrets = Arc::new(SecureSecretFactory);
    let matrix_identities = build_matrix_identity_provisioner(config, request_timeout)?;
    let service = Arc::new(AuthenticationService::new(
        AuthenticationDependencies {
            oidc,
            login_attempts: repositories.clone(),
            login_completion: repositories.clone(),
            sessions: repositories.clone(),
            suspensions: repositories.clone(),
            secrets: secrets.clone(),
            identifiers: system_runtime.clone(),
            clock: system_runtime.clone(),
        },
        policy,
    ));
    let state = build_authentication_http_state(service.clone(), authentication_config)?;
    let telemetry_state = build_frontend_telemetry_state(config, service.clone(), metrics.clone());
    let devices = build_device_authorization(
        authentication_config,
        repositories.clone(),
        secrets.clone(),
        system_runtime.clone(),
        matrix_identities.clone(),
    )?;
    let device_state = DeviceHttpState::new(
        DeviceHttpDependencies {
            devices: devices.clone(),
            assertion_verifier: device_oidc,
            authentication: service.clone(),
            secrets: secrets.clone(),
        },
        &authentication_config.frontend_origin,
    );
    let content_runtime =
        content_runtime::initialize(content_runtime::ContentRuntimeDependencies {
            config: &config.content,
            matrix_base_url: config.dependencies.matrix_base_url.as_str(),
            matrix_request_timeout: request_timeout,
            repositories: repositories.clone(),
            system_runtime: system_runtime.clone(),
            authentication: service.clone(),
            devices: devices.clone(),
            secrets: secrets.clone(),
            matrix_identities: matrix_identities.clone(),
            frontend_origin: &authentication_config.frontend_origin,
        })
        .await?;
    let (content_routes, content_cleanup, matrix_authority) = content_runtime.into_parts();
    let account_lifecycle = build_account_lifecycle_service(
        &config.account_lifecycle,
        repositories.clone(),
        system_runtime.clone(),
    )?;
    let account_deletion = build_account_deletion_worker(
        &config.account_lifecycle,
        &config.dependencies.matrix_base_url,
        &authentication_config.matrix_server_name,
        request_timeout,
        repositories.clone(),
        system_runtime.clone(),
    )?;
    let operational_metrics = start_operational_metrics(config, &repositories, metrics)?;
    let account_state = build_account_http_state(config, account_lifecycle, service.clone());
    let agent_features = build_agent_feature_states(
        config,
        request_timeout,
        AgentFeatureDependencies {
            repositories,
            system_runtime,
            secrets,
            matrix_identities,
            authentication: service,
            devices,
            matrix_authority,
        },
    )?;
    let routes = compose_identity_routes(
        state,
        telemetry_state,
        account_state,
        device_state,
        agent_features,
        content_routes,
    );
    Ok(IdentityRuntime {
        routes,
        content_cleanup,
        account_deletion,
        operational_metrics,
    })
}

fn compose_identity_routes(
    authentication: AuthenticationHttpState,
    telemetry: FrontendTelemetryHttpState,
    accounts: AccountHttpState,
    devices: DeviceHttpState,
    agents: AgentFeatureHttpStates,
    content: Router,
) -> Router {
    features::authentication::router(authentication)
        .merge(features::telemetry::router(telemetry))
        .merge(features::accounts::router(accounts))
        .merge(features::devices::router(devices))
        .merge(features::agents::router(agents.agents))
        .merge(features::agent_instances::router(agents.instances))
        .merge(features::handoffs::router(agents.handoffs))
        .merge(features::lobbies::router(agents.lobbies))
        .merge(features::private_rooms::router(agents.private_rooms))
        .merge(features::direct_sessions::router(agents.direct_sessions))
        .merge(features::agent_cards::router(agents.cards))
        .merge(features::automation::router(agents.automation))
        .merge(features::moderation::router(agents.moderation))
        .merge(content)
}

fn start_operational_metrics(
    config: &ControlPlaneConfig,
    repositories: &PostgresRepositories,
    metrics: TelemetryMetrics,
) -> Result<operational_metrics::OperationalMetricsRuntime, StartupError> {
    operational_metrics::OperationalMetricsRuntime::start(
        repositories.pool().clone(),
        metrics,
        config.observability.operational_sample_interval,
    )
    .map_err(|error| StartupError::new("startup.invalid_observability_config", error.to_string()))
}

fn build_frontend_telemetry_state(
    config: &ControlPlaneConfig,
    authentication: Arc<AuthenticationService>,
    metrics: TelemetryMetrics,
) -> FrontendTelemetryHttpState {
    FrontendTelemetryHttpState::new(
        authentication,
        &config.authentication.frontend_origin,
        metrics,
    )
}

fn build_account_http_state(
    config: &ControlPlaneConfig,
    accounts: Arc<AccountLifecycleService>,
    authentication: Arc<AuthenticationService>,
) -> AccountHttpState {
    AccountHttpState::new(
        accounts,
        authentication,
        &config.authentication.frontend_origin,
    )
}

fn build_account_deletion_worker(
    config: &AccountLifecycleConfig,
    matrix_base_url: &url::Url,
    matrix_server_name: &str,
    request_timeout: Duration,
    repositories: Arc<PostgresRepositories>,
    runtime: Arc<SystemRuntime>,
) -> Result<account_deletion::AccountDeletionRuntime, StartupError> {
    let matrix = Arc::new(
        SynapseAccountLifecycleGateway::new(
            SynapseAccountLifecycleConfiguration::new(
                matrix_base_url.as_str(),
                matrix_server_name.to_owned(),
                SecretValue::new(config.matrix_admin_access_token.expose().to_owned()).map_err(
                    |_| {
                        StartupError::new(
                            "startup.invalid_account_lifecycle_config",
                            "Matrix 管理令牌无效".to_owned(),
                        )
                    },
                )?,
                request_timeout,
            )
            .map_err(|error| {
                StartupError::new(
                    "startup.invalid_account_lifecycle_config",
                    error.to_string(),
                )
            })?,
        )
        .map_err(|error| {
            StartupError::new(
                "startup.invalid_account_lifecycle_config",
                error.to_string(),
            )
        })?,
    );
    let worker = Arc::new(AccountDeletionWorker::new(
        AccountDeletionWorkerDependencies {
            repository: repositories,
            matrix,
            clock: runtime,
            lease_duration: domain_duration(config.lease_duration)?,
            initial_retry_delay: domain_duration(config.retry_initial)?,
            maximum_retry_delay: domain_duration(config.retry_maximum)?,
        },
    ));
    account_deletion::AccountDeletionRuntime::start(worker, config.worker_interval).map_err(
        |error| {
            StartupError::new(
                "startup.invalid_account_lifecycle_config",
                error.to_string(),
            )
        },
    )
}

fn build_account_lifecycle_service(
    config: &AccountLifecycleConfig,
    repositories: Arc<PostgresRepositories>,
    runtime: Arc<SystemRuntime>,
) -> Result<Arc<AccountLifecycleService>, StartupError> {
    let receipts = Arc::new(
        HmacAccountDeletionReceiptIssuer::new(config.receipt_secret.expose().as_bytes().to_vec())
            .map_err(|error| {
            StartupError::new(
                "startup.invalid_account_lifecycle_config",
                error.to_string(),
            )
        })?,
    );
    Ok(Arc::new(AccountLifecycleService::new(
        AccountLifecycleDependencies {
            repository: repositories,
            receipts,
            clock: runtime,
        },
    )))
}

fn build_agent_feature_states(
    config: &ControlPlaneConfig,
    request_timeout: Duration,
    dependencies: AgentFeatureDependencies,
) -> Result<AgentFeatureHttpStates, StartupError> {
    let agents = build_agent_management(
        dependencies.repositories.clone(),
        dependencies.secrets.clone(),
        dependencies.system_runtime.clone(),
        dependencies.matrix_identities.clone(),
    );
    let verification = build_agent_instance_verification(
        dependencies.repositories.clone(),
        dependencies.system_runtime.clone(),
    );
    let instances = build_agent_instance_management(
        dependencies.repositories.clone(),
        dependencies.system_runtime.clone(),
        dependencies.matrix_identities.clone(),
    );
    let cards = build_agent_card_management(
        dependencies.repositories.clone(),
        dependencies.system_runtime.clone(),
        request_timeout,
    );
    let handoffs = Arc::new(HandoffAccessService::new(HandoffAccessDependencies {
        access: dependencies.repositories.clone(),
        clock: dependencies.system_runtime.clone(),
    }));
    let entries = build_agent_lobby_entry(
        &config.lobby,
        dependencies.repositories.clone(),
        dependencies.system_runtime.clone(),
        dependencies.matrix_identities.clone(),
    )?;
    let lobby_observation = Arc::new(PublicLobbyObservationService::new(
        dependencies.repositories.clone(),
    ));
    let private_rooms = Arc::new(PrivateRoomService::new(PrivateRoomDependencies {
        store: dependencies.repositories.clone(),
        matrix_provisioner: dependencies.matrix_identities.clone(),
        matrix: dependencies.matrix_identities.clone(),
        principals: dependencies.repositories.clone(),
        trusted_matrix_readers: vec![content_authority_matrix_user(config)?],
        identifiers: dependencies.system_runtime.clone(),
        clock: dependencies.system_runtime.clone(),
    }));
    let automation = build_automation_http_state(config, &dependencies);
    let moderation = build_moderation_management(&dependencies);
    let direct_sessions = build_direct_session_management(&dependencies);
    Ok(AgentFeatureHttpStates {
        agents: AgentHttpState::new(
            AgentHttpDependencies {
                agents,
                verification,
                authentication: dependencies.authentication.clone(),
                devices: dependencies.devices.clone(),
                secrets: dependencies.secrets.clone(),
            },
            &config.authentication.frontend_origin,
        ),
        instances: AgentInstanceHttpState::new(
            AgentInstanceHttpStateDependencies {
                instances,
                authentication: dependencies.authentication.clone(),
            },
            &config.authentication.frontend_origin,
        ),
        cards: AgentCardHttpState::new(AgentCardHttpDependencies {
            cards,
            devices: dependencies.devices.clone(),
            secrets: dependencies.secrets.clone(),
        }),
        handoffs: HandoffHttpState::new(HandoffHttpDependencies {
            handoffs,
            devices: dependencies.devices.clone(),
            secrets: dependencies.secrets.clone(),
        }),
        lobbies: LobbyHttpState::new(LobbyHttpDependencies {
            entries,
            directory: dependencies.repositories.clone(),
            observation: lobby_observation,
            authentication: dependencies.authentication.clone(),
            devices: dependencies.devices.clone(),
            secrets: dependencies.secrets.clone(),
        }),
        private_rooms: PrivateRoomHttpState::new(
            private_rooms,
            dependencies.authentication.clone(),
            &config.authentication.frontend_origin,
        ),
        direct_sessions: DirectSessionHttpState::new(
            direct_sessions,
            dependencies.authentication.clone(),
            &config.authentication.frontend_origin,
        ),
        automation,
        moderation: ModerationHttpState::new(
            moderation,
            dependencies.authentication,
            &config.authentication.frontend_origin,
        ),
    })
}

fn build_moderation_management(dependencies: &AgentFeatureDependencies) -> Arc<ModerationService> {
    Arc::new(ModerationService::new(ModerationDependencies {
        repository: dependencies.repositories.clone(),
        authority: dependencies.repositories.clone(),
        effects: dependencies.matrix_identities.clone(),
        identifiers: dependencies.system_runtime.clone(),
        clock: dependencies.system_runtime.clone(),
        report_policy: agent_room_application::ports::ModerationReportPolicy {
            maximum_reports: 5,
            window: DurationMillis::new(10 * 60 * 1_000).expect("固定举报限速窗口必须有效"),
        },
    }))
}

fn build_direct_session_management(
    dependencies: &AgentFeatureDependencies,
) -> Arc<DirectSessionService> {
    Arc::new(DirectSessionService::new(DirectSessionDependencies {
        store: dependencies.repositories.clone(),
        agents: dependencies.repositories.clone(),
        matrix: dependencies.matrix_identities.clone(),
        identifiers: dependencies.system_runtime.clone(),
        clock: dependencies.system_runtime.clone(),
    }))
}

fn build_automation_http_state(
    config: &ControlPlaneConfig,
    dependencies: &AgentFeatureDependencies,
) -> AutomationHttpState {
    let automation = Arc::new(AutomationService::new(AutomationDependencies {
        grants: dependencies.repositories.clone(),
        authority: dependencies.repositories.clone(),
        matrix_authority: dependencies.matrix_authority.clone(),
        clock: dependencies.system_runtime.clone(),
    }));
    AutomationHttpState::new(
        AutomationHttpDependencies {
            automation,
            authentication: dependencies.authentication.clone(),
            devices: dependencies.devices.clone(),
            secrets: dependencies.secrets.clone(),
        },
        &config.authentication.frontend_origin,
    )
}

fn content_authority_matrix_user(
    config: &ControlPlaneConfig,
) -> Result<MatrixUserId, StartupError> {
    let localpart = MatrixAgentLocalpart::from_agent_id(config.content.matrix_authority_agent_id);
    MatrixUserId::new(format!(
        "@{}:{}",
        localpart.as_str(),
        config.authentication.matrix_server_name
    ))
    .map_err(|_| {
        StartupError::new(
            "startup.invalid_content_matrix_identity",
            "内容授权 Matrix 用户标识无效".to_owned(),
        )
    })
}

fn build_agent_lobby_entry(
    config: &LobbyConfig,
    repositories: Arc<PostgresRepositories>,
    system_runtime: Arc<SystemRuntime>,
    matrix: Arc<MatrixApplicationServiceProvisioner>,
) -> Result<Arc<AgentLobbyEntryService>, StartupError> {
    let reservation_lifetime = domain_duration(config.reservation_lifetime)?;
    let provisioning_lease_lifetime = domain_duration(config.provisioning_lease_lifetime)?;
    let join_policy = LobbyJoinPolicy::new(reservation_lifetime)
        .map_err(|error| StartupError::new("startup.invalid_lobby_config", error.to_string()))?;
    let provisioning_policy = LobbyProvisioningPolicy::new(provisioning_lease_lifetime)
        .map_err(|error| StartupError::new("startup.invalid_lobby_config", error.to_string()))?;
    let provisioning = Arc::new(LobbyProvisioningService::new(
        LobbyProvisioningDependencies {
            store: repositories.clone(),
            matrix: matrix.clone(),
            identifiers: system_runtime.clone(),
            clock: system_runtime.clone(),
        },
        provisioning_policy,
    ));
    Ok(Arc::new(AgentLobbyEntryService::new(
        AgentLobbyEntryDependencies {
            access: repositories.clone(),
            allocations: repositories,
            memberships: matrix,
            provisioning,
            identifiers: system_runtime.clone(),
            clock: system_runtime,
        },
        join_policy,
    )))
}

fn build_authentication_http_state(
    service: Arc<AuthenticationService>,
    config: &AuthenticationConfig,
) -> Result<AuthenticationHttpState, StartupError> {
    AuthenticationHttpState::new(
        service,
        config.issuer_url.clone(),
        config.frontend_origin.clone(),
        config.login_attempt_ttl,
        config.web_session_ttl,
    )
    .map_err(|error| StartupError::new("startup.invalid_authentication_config", error.to_string()))
}

fn build_device_authorization(
    config: &AuthenticationConfig,
    repositories: Arc<PostgresRepositories>,
    secrets: Arc<SecureSecretFactory>,
    system_runtime: Arc<SystemRuntime>,
    matrix: Arc<MatrixApplicationServiceProvisioner>,
) -> Result<Arc<DeviceAuthorizationService>, StartupError> {
    let policy = device_authorization_policy(config)?;
    Ok(Arc::new(DeviceAuthorizationService::new(
        DeviceAuthorizationDependencies {
            registrations: repositories.clone(),
            sessions: repositories.clone(),
            proof_nonces: repositories.clone(),
            proof_verifier: Arc::new(Ed25519DeviceProofVerifier),
            devices: repositories.clone(),
            revocations: repositories.clone(),
            matrix_cleanup: repositories,
            matrix,
            secrets,
            identifiers: system_runtime.clone(),
            clock: system_runtime,
        },
        policy,
    )))
}

fn build_agent_card_management(
    repositories: Arc<PostgresRepositories>,
    system_runtime: Arc<SystemRuntime>,
    request_timeout: Duration,
) -> Arc<AgentCardService> {
    let documents: Arc<dyn HttpsDocumentClient> = Arc::new(PinnedHttpsClient::new(
        Arc::new(SystemDnsResolver),
        PinnedHttpsClientConfiguration {
            connect_timeout: request_timeout.min(Duration::from_secs(3)),
            request_timeout,
        },
    ));
    let source = Arc::new(RemoteAgentCardSource::new(
        documents,
        AgentCardNormalizer::default(),
    ));
    Arc::new(AgentCardService::new(AgentCardDependencies {
        agents: repositories.clone(),
        memberships: repositories.clone(),
        source,
        snapshots: repositories,
        identifiers: system_runtime.clone(),
        clock: system_runtime,
    }))
}

fn build_agent_management(
    repositories: Arc<PostgresRepositories>,
    secrets: Arc<SecureSecretFactory>,
    system_runtime: Arc<SystemRuntime>,
    matrix_identities: Arc<MatrixApplicationServiceProvisioner>,
) -> Arc<AgentManagementService> {
    Arc::new(AgentManagementService::new(AgentManagementDependencies {
        creations: repositories.clone(),
        agents: repositories.clone(),
        memberships: repositories.clone(),
        membership_changes: repositories.clone(),
        instances: repositories,
        matrix_identities,
        secrets,
        identifiers: system_runtime.clone(),
        clock: system_runtime,
    }))
}

fn build_agent_instance_verification(
    repositories: Arc<PostgresRepositories>,
    system_runtime: Arc<SystemRuntime>,
) -> Arc<AgentInstanceVerificationService> {
    Arc::new(AgentInstanceVerificationService::new(
        AgentInstanceVerificationDependencies {
            records: repositories,
            clock: system_runtime,
        },
    ))
}

fn build_agent_instance_management(
    repositories: Arc<PostgresRepositories>,
    system_runtime: Arc<SystemRuntime>,
    matrix: Arc<MatrixApplicationServiceProvisioner>,
) -> Arc<AgentInstanceManagementService> {
    Arc::new(AgentInstanceManagementService::new(
        AgentInstanceManagementDependencies {
            instances: repositories.clone(),
            revocations: repositories.clone(),
            matrix_cleanup: repositories,
            matrix,
            identifiers: system_runtime.clone(),
            clock: system_runtime,
        },
    ))
}

fn build_matrix_identity_provisioner(
    config: &ControlPlaneConfig,
    request_timeout: Duration,
) -> Result<Arc<MatrixApplicationServiceProvisioner>, StartupError> {
    let matrix_token = SecretValue::new(
        config
            .agent_identity
            .matrix_application_service_token
            .expose()
            .to_owned(),
    )
    .map_err(|_| {
        StartupError::new(
            "startup.invalid_agent_identity_config",
            "Matrix Application Service Token 无效".to_owned(),
        )
    })?;
    let matrix_configuration = MatrixApplicationServiceConfiguration::new(
        config.dependencies.matrix_base_url.as_str(),
        config.authentication.matrix_server_name.clone(),
        matrix_token,
        request_timeout,
    )
    .map_err(|error| {
        StartupError::new("startup.invalid_agent_identity_config", error.to_string())
    })?;
    let matrix_identities = Arc::new(
        MatrixApplicationServiceProvisioner::new(matrix_configuration).map_err(|error| {
            StartupError::new("startup.invalid_agent_identity_config", error.to_string())
        })?,
    );
    Ok(matrix_identities)
}

fn build_web_oidc(
    config: &AuthenticationConfig,
    request_timeout: Duration,
) -> Result<Arc<DiscoveredOidcGateway>, StartupError> {
    let client_secret =
        SecretValue::new(config.client_secret.expose().to_owned()).map_err(|_| {
            StartupError::new(
                "startup.invalid_authentication_config",
                "OIDC 客户端密钥无效".to_owned(),
            )
        })?;
    DiscoveredOidcGateway::new(OidcAdapterConfig {
        issuer_url: config.issuer_url.to_string(),
        client_id: config.client_id.clone(),
        client_secret,
        redirect_url: config.redirect_url.to_string(),
        request_timeout,
    })
    .map(Arc::new)
    .map_err(|error| StartupError::new("startup.invalid_authentication_config", error.to_string()))
}

fn build_device_oidc(
    config: &AuthenticationConfig,
    request_timeout: Duration,
) -> Result<Arc<DiscoveredOidcDeviceGrant>, StartupError> {
    DiscoveredOidcDeviceGrant::new(OidcDeviceGrantConfig {
        issuer_url: config.issuer_url.to_string(),
        client_id: config.device_client_id.clone(),
        request_timeout,
        maximum_polling_duration: config.device_authorization_maximum_age,
    })
    .map(Arc::new)
    .map_err(|error| {
        StartupError::new(
            "startup.invalid_device_authentication_config",
            error.to_string(),
        )
    })
}

fn authentication_policy(
    config: &AuthenticationConfig,
) -> Result<AuthenticationPolicy, StartupError> {
    AuthenticationPolicy::new(
        domain_duration(config.login_attempt_ttl)?,
        domain_duration(config.web_session_ttl)?,
        domain_duration(config.recent_authentication_window)?,
        domain_duration(config.allowed_clock_skew)?,
        config.matrix_server_name.clone(),
    )
    .map_err(|_| {
        StartupError::new(
            "startup.invalid_authentication_config",
            "Matrix 服务名无效".to_owned(),
        )
    })
}

fn device_authorization_policy(
    config: &AuthenticationConfig,
) -> Result<DeviceAuthorizationPolicy, StartupError> {
    DeviceAuthorizationPolicy::new(
        domain_duration(config.device_access_token_ttl)?,
        domain_duration(config.device_refresh_token_ttl)?,
        domain_duration(config.device_proof_maximum_age)?,
        domain_duration(config.allowed_clock_skew)?,
        domain_duration(config.device_authorization_maximum_age)?,
        config.matrix_server_name.clone(),
    )
    .map_err(|_| {
        StartupError::new(
            "startup.invalid_device_authentication_config",
            "设备授权时限或 Matrix 服务名无效".to_owned(),
        )
    })
}

fn domain_duration(duration: Duration) -> Result<DurationMillis, StartupError> {
    let milliseconds = u64::try_from(duration.as_millis()).map_err(|_| {
        StartupError::new(
            "startup.invalid_authentication_config",
            "认证时限超出范围".to_owned(),
        )
    })?;
    DurationMillis::new(milliseconds).map_err(|_| {
        StartupError::new(
            "startup.invalid_authentication_config",
            "认证时限必须大于零".to_owned(),
        )
    })
}

#[derive(Debug)]
pub struct StartupError {
    code: &'static str,
    message: String,
}

impl StartupError {
    pub(crate) fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}：{}", self.code, self.message)
    }
}

impl Error for StartupError {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_application::{
        health::{
            DependencyKind, DependencyProbe, ProbeFailure, ProbeFailureKind, ProbeResult,
            ReadinessService,
        },
        ports::PortFuture,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{HeaderValue, Method, Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::{Uuid, Version};

    use super::{
        build_router, correlation::CORRELATION_ID_HEADER, telemetry_metrics::TelemetryMetrics,
    };

    const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

    struct StaticProbe {
        dependency: DependencyKind,
        result: ProbeResult,
    }

    impl DependencyProbe for StaticProbe {
        fn dependency(&self) -> DependencyKind {
            self.dependency
        }

        fn check<'a>(&'a self, _correlation_id: &'a str) -> PortFuture<'a, ProbeResult> {
            Box::pin(async move { self.result })
        }
    }

    fn probe(dependency: DependencyKind, result: ProbeResult) -> Arc<dyn DependencyProbe> {
        Arc::new(StaticProbe { dependency, result })
    }

    fn router_with(matrix_result: ProbeResult) -> axum::Router {
        let readiness = ReadinessService::new(vec![
            probe(DependencyKind::PostgreSql, Ok(())),
            probe(DependencyKind::Matrix, matrix_result),
            probe(DependencyKind::ObjectStore, Ok(())),
        ])
        .expect("测试探针配置有效");
        build_router(
            Arc::new(readiness),
            axum::Router::new(),
            &url::Url::parse(FRONTEND_ORIGIN).expect("测试前端 Origin 有效"),
            TelemetryMetrics::new(),
        )
        .expect("测试 CORS 配置有效")
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), 128 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&bytes).expect("响应正文必须是 JSON")
    }

    #[tokio::test]
    async fn 存活端点生成并回传_uuidv7_关联标识() {
        let response = router_with(Ok(()))
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        let header = response
            .headers()
            .get(CORRELATION_ID_HEADER)
            .expect("响应必须带关联 ID")
            .to_str()
            .expect("关联 ID 是 ASCII")
            .to_owned();
        assert_eq!(
            Uuid::parse_str(&header)
                .expect("关联 ID 是 UUID")
                .get_version(),
            Some(Version::SortRand)
        );
        let body = body_json(response).await;
        assert_eq!(body["status"], "live");
        assert_eq!(body["correlationId"], header);
    }

    #[tokio::test]
    async fn 就绪端点用_503_指出具体降级层() {
        let response = router_with(Err(ProbeFailure::new(ProbeFailureKind::Timeout)))
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(response).await;
        assert_eq!(body["status"], "degraded");
        let matrix = body["dependencies"]
            .as_array()
            .expect("依赖列表存在")
            .iter()
            .find(|dependency| dependency["name"] == "matrix")
            .expect("Matrix 层存在");
        assert_eq!(matrix["status"], "unavailable");
        assert_eq!(matrix["failure"], "timeout");
    }

    #[tokio::test]
    async fn 能力端点直接使用协议生成类型() {
        let response = router_with(Ok(()))
            .oneshot(
                Request::builder()
                    .uri("/capabilities")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let expected: Value = serde_json::from_str(include_str!(
            "../../../packages/protocol/fixtures/valid/capabilities.json"
        ))
        .expect("协议夹具有效");
        assert_eq!(body, expected);
    }

    #[tokio::test]
    async fn 未知路由也返回结构化错误和关联标识() {
        let correlation_id = Uuid::now_v7();
        let response = router_with(Ok(()))
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .header(CORRELATION_ID_HEADER, correlation_id.to_string())
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = body_json(response).await;
        assert_eq!(body["code"], "http.route_not_found");
        assert_eq!(body["correlationId"], correlation_id.to_string());
    }

    #[tokio::test]
    async fn 浏览器预检只允许配置的前端_origin_并携带凭据() {
        let response = router_with(Ok(()))
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/auth/session")
                    .header(header::ORIGIN, FRONTEND_ORIGIN)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static(FRONTEND_ORIGIN))
        );
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
            Some(&HeaderValue::from_static("true"))
        );
    }

    #[tokio::test]
    async fn 浏览器预检拒绝未配置的_origin() {
        let response = router_with(Ok(()))
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/auth/session")
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        assert_ne!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("https://evil.example"))
        );
    }
}

#[cfg(test)]
mod real_dependency_tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use url::Url;

    use super::{
        build_router,
        config::{ControlPlaneConfig, DependencyConfig},
        features::health::HealthRuntime,
        telemetry_metrics::TelemetryMetrics,
    };

    async fn ready_response(config: &DependencyConfig) -> (StatusCode, Value) {
        let runtime = HealthRuntime::initialize(config).expect("真实依赖探针可初始化");
        let frontend_origin =
            Url::parse("https://app.agent-room.test").expect("测试前端 Origin 有效");
        let response = build_router(
            runtime.readiness.clone(),
            axum::Router::new(),
            &frontend_origin,
            TelemetryMetrics::new(),
        )
        .expect("测试 CORS 配置有效")
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("请求有效"),
        )
        .await
        .expect("路由执行成功");
        runtime.shutdown().await;

        let status = response.status();
        let bytes = to_bytes(response.into_body(), 128 * 1_024)
            .await
            .expect("响应正文可读取");
        let body = serde_json::from_slice(&bytes).expect("响应正文必须是 JSON");
        (status, body)
    }

    fn assert_only_dependency_failed(body: &Value, expected: &str) {
        let failed = body["dependencies"]
            .as_array()
            .expect("依赖列表存在")
            .iter()
            .filter(|dependency| dependency["status"] == "unavailable")
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["name"], expected);
    }

    #[tokio::test]
    #[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
    async fn 真实依赖正常及逐层断连都能准确映射() {
        let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");

        let (status, body) = ready_response(&config.dependencies).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ready");

        let mut database_down = config.dependencies.clone();
        database_down.database.port = 1;
        let (status, body) = ready_response(&database_down).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_only_dependency_failed(&body, "postgresql");

        let mut matrix_down = config.dependencies.clone();
        matrix_down.matrix_base_url = Url::parse("http://127.0.0.1:1").expect("测试 URL 有效");
        let (status, body) = ready_response(&matrix_down).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_only_dependency_failed(&body, "matrix");

        let mut object_store_down = config.dependencies.clone();
        object_store_down.object_store_health_url =
            Url::parse("http://127.0.0.1:1").expect("测试 URL 有效");
        let (status, body) = ready_response(&object_store_down).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_only_dependency_failed(&body, "object_store");
    }
}
