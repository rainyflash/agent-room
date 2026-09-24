//! 网页在成员列表、名册和消息头上标出“网络 Agent”（ADR 0010）：给一批 Agent ID，回答其中哪些是
//! 只凭网络接入的 Agent。停用的也算，同一个 Agent 的答案不会变，网页可以一直缓存。

use std::{collections::BTreeSet, sync::Arc};

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::NetworkAgentLookup,
};
use agent_room_domain::ids::AgentId;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store},
};

/// 一次最多问这么多个；网页按这个数分批。
const MAX_CANDIDATES: usize = 100;
const SCHEMA_VERSION: u8 = 1;

#[derive(Clone)]
pub(crate) struct NetworkAgentLookupHttpState {
    pub(crate) lookup: Arc<dyn NetworkAgentLookup>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
}

pub(crate) fn router(state: NetworkAgentLookupHttpState) -> Router {
    Router::new()
        .route("/network-agents/lookup", get(lookup))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LookupQuery {
    /// 逗号分隔的 Agent ID。
    agent_ids: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupResponse {
    schema_version: u8,
    network_agent_ids: Vec<String>,
}

async fn lookup(
    State(state): State<NetworkAgentLookupHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
    query: Result<Query<LookupQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        return response;
    }
    let Some(candidates) = query
        .ok()
        .and_then(|Query(query)| parse_candidates(&query.agent_ids))
    else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_lookup",
                ErrorCategory::Validation,
                "agentIds 应为逗号分隔的 1 到 100 个 Agent ID。",
                correlation_id,
            )
            .into_response(),
        );
    };
    match state.lookup.network_agent_ids(&candidates).await {
        Ok(found) => no_store(
            Json(LookupResponse {
                schema_version: SCHEMA_VERSION,
                network_agent_ids: found.iter().map(ToString::to_string).collect(),
            })
            .into_response(),
        ),
        Err(error) => {
            tracing::warn!(
                operation = "network_agent.lookup",
                kind = ?error.kind(),
                "查不了哪些是网络 Agent"
            );
            no_store(
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "network_agent.lookup_unavailable",
                    ErrorCategory::DependencyUnavailable,
                    "暂时查不了哪些是网络 Agent。",
                    correlation_id,
                )
                .into_response(),
            )
        }
    }
}

/// 去重后 1 到 100 个合法的 Agent ID；有一个不合法就整体拒绝。
fn parse_candidates(value: &str) -> Option<Vec<AgentId>> {
    let mut ids = BTreeSet::new();
    for part in value.split(',') {
        ids.insert(Uuid::parse_str(part.trim()).ok()?);
    }
    if ids.len() > MAX_CANDIDATES {
        return None;
    }
    Some(ids.into_iter().map(AgentId::from_uuid).collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use agent_room_application::{
        authentication::{
            AuthenticatedPrincipal, AuthenticationResult, BeginLogin, CompleteLogin,
            LoginCompletion, LoginRedirect,
        },
        persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
        ports::{PortFuture, SecretValue},
    };
    use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
        middleware,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";
    const NETWORK_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
    const LOCAL_AGENT: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";

    #[derive(Default)]
    struct FakeLookup {
        asked: Mutex<Vec<Vec<AgentId>>>,
        unavailable: bool,
    }

    impl NetworkAgentLookup for FakeLookup {
        fn network_agent_ids<'a>(
            &'a self,
            candidates: &'a [AgentId],
        ) -> PortFuture<'a, RepositoryResult<Vec<AgentId>>> {
            self.asked.lock().unwrap().push(candidates.to_vec());
            let result = if self.unavailable {
                Err(RepositoryError::new(
                    "network_agent.lookup",
                    RepositoryErrorKind::Unavailable,
                ))
            } else {
                Ok(candidates
                    .iter()
                    .copied()
                    .filter(|id| id.to_string() == NETWORK_AGENT)
                    .collect())
            };
            Box::pin(async move { result })
        }
    }

    struct FakeAuthentication;

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            _request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            Box::pin(async { unreachable!("查网络 Agent 不会开始登录") })
        }

        fn complete_login<'a>(
            &'a self,
            _request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            Box::pin(async { unreachable!("查网络 Agent 不会完成登录") })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            assert_eq!(session_secret.expose(), "session-secret");
            assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
            Box::pin(async {
                Ok(AuthenticatedPrincipal {
                    principal_id: PrincipalId::from_uuid(
                        Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e42").unwrap(),
                    ),
                    matrix_user_id: "@viewer:matrix.agent-room.test".to_owned(),
                    display_name: "观众".to_owned(),
                    locale: "zh-CN".to_owned(),
                    authenticated_at: UtcMillis::new(1_700_000_000_000).unwrap(),
                    expires_at: UtcMillis::new(1_700_000_060_000).unwrap(),
                    recently_authenticated: true,
                })
            })
        }

        fn logout<'a>(
            &'a self,
            _session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("查网络 Agent 不会退出登录") })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("查网络 Agent 不会暂停主体") })
        }
    }

    fn app(lookup: Arc<FakeLookup>) -> Router {
        router(NetworkAgentLookupHttpState {
            lookup,
            authentication: Arc::new(FakeAuthentication),
        })
        .layer(middleware::from_fn(crate::correlation::attach))
    }

    fn request(query: &str, cookie: bool) -> Request<Body> {
        let mut request = Request::builder().uri(format!("/network-agents/lookup{query}"));
        if cookie {
            request = request.header(header::COOKIE, SESSION_COOKIE);
        }
        request.body(Body::empty()).unwrap()
    }

    async fn body_json(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn 登录后按去重的_id_查_只回答其中的网络_agent() {
        let lookup = Arc::new(FakeLookup::default());

        let response = app(lookup.clone())
            .oneshot(request(
                &format!("?agentIds={LOCAL_AGENT},{NETWORK_AGENT},%20{LOCAL_AGENT}"),
                true,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = body_json(response).await;
        assert_eq!(body["schemaVersion"], 1);
        assert_eq!(body["networkAgentIds"], serde_json::json!([NETWORK_AGENT]));
        let asked = lookup.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].len(), 2, "重复的 ID 只问一次");
    }

    #[tokio::test]
    async fn 没登录不查() {
        let lookup = Arc::new(FakeLookup::default());

        let response = app(lookup.clone())
            .oneshot(request(&format!("?agentIds={NETWORK_AGENT}"), false))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(lookup.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn 参数不对时说明该怎么写_不查() {
        let too_many = (0..=MAX_CANDIDATES)
            .map(|_| Uuid::now_v7().to_string())
            .collect::<Vec<_>>()
            .join(",");
        for query in [
            String::new(),
            "?agentIds=".to_owned(),
            "?agentIds=not-an-id".to_owned(),
            format!("?agentIds={NETWORK_AGENT},,{LOCAL_AGENT}"),
            format!("?agentIds={NETWORK_AGENT}&extra=1"),
            format!("?agentIds={too_many}"),
        ] {
            let lookup = Arc::new(FakeLookup::default());

            let response = app(lookup.clone())
                .oneshot(request(&query, true))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
            assert_eq!(
                body_json(response).await["code"],
                "network_agent.invalid_lookup"
            );
            assert!(lookup.asked.lock().unwrap().is_empty(), "{query}");
        }
    }

    #[tokio::test]
    async fn 数据库不可用时回答暂时查不了() {
        let lookup = Arc::new(FakeLookup {
            unavailable: true,
            ..FakeLookup::default()
        });

        let response = app(lookup)
            .oneshot(request(&format!("?agentIds={NETWORK_AGENT}"), true))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.lookup_unavailable"
        );
    }
}
