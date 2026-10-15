//! 房间里的人叫什么、Agent 是谁的（`specs/room-people/design.md`）。
//!
//! 都是新的接口，不往私人房间的回答里加字段：网页对它 `.strict()` 校验，旧版桌面端多看到一个
//! 字段整个房间都读不出来。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::{RoomAgentKind, RoomAgentOwnerRecord, RoomPersonName},
    room_people::{
        ListRoomPeople, LookupRoomAgentOwners, MAX_AGENT_OWNER_CANDIDATES, RoomPeopleFailure,
        RoomPeopleUseCases,
    },
};
use agent_room_domain::ids::{AgentId, RoomCatalogId};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{authenticate_session, no_store},
        resource_ids::parse_uuid_v7,
    },
};

#[derive(Clone)]
pub(crate) struct RoomPeopleHttpState {
    pub(crate) people: Arc<dyn RoomPeopleUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
}

pub(crate) fn router(state: RoomPeopleHttpState) -> Router {
    Router::new()
        .route("/private-rooms/{catalog_id}/people", get(list_people))
        .route("/rooms/{catalog_id}/agent-owners", get(lookup_agent_owners))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentOwnersQuery {
    /// 逗号分隔的 Agent ID，和 `/network-agents/lookup` 一样。
    agent_ids: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeopleResponse {
    people: Vec<PersonResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonResponse {
    principal_id: String,
    display_name: String,
}

impl From<RoomPersonName> for PersonResponse {
    fn from(person: RoomPersonName) -> Self {
        Self {
            principal_id: person.principal_id.to_string(),
            display_name: person.display_name,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentOwnersResponse {
    agents: Vec<AgentOwnerResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentOwnerResponse {
    agent_id: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<PersonResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admitted_by: Option<PersonResponse>,
}

impl From<RoomAgentOwnerRecord> for AgentOwnerResponse {
    fn from(record: RoomAgentOwnerRecord) -> Self {
        Self {
            agent_id: record.agent_id.to_string(),
            kind: match record.kind {
                RoomAgentKind::Local => "local",
                RoomAgentKind::Network => "network",
            },
            owner: record.owner.map(PersonResponse::from),
            admitted_by: record.admitted_by.map(PersonResponse::from),
        }
    }
}

async fn list_people(
    State(state): State<RoomPeopleHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog_id): Path<String>,
    jar: CookieJar,
) -> Response {
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(invalid("room_people.invalid_catalog", correlation_id));
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .people
        .people(ListRoomPeople { actor, catalog_id })
        .await
    {
        Ok(people) => no_store(
            Json(PeopleResponse {
                people: people.into_iter().map(PersonResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(failure_response(failure, correlation_id)),
    }
}

async fn lookup_agent_owners(
    State(state): State<RoomPeopleHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog_id): Path<String>,
    jar: CookieJar,
    query: Result<Query<AgentOwnersQuery>, QueryRejection>,
) -> Response {
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(invalid("room_people.invalid_catalog", correlation_id));
    };
    let Some(agent_ids) = query
        .ok()
        .and_then(|Query(query)| parse_agent_ids(&query.agent_ids))
    else {
        return no_store(invalid("room_people.invalid_agent_ids", correlation_id));
    };
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state
        .people
        .agent_owners(LookupRoomAgentOwners {
            actor,
            catalog_id,
            agent_ids,
        })
        .await
    {
        Ok(agents) => no_store(
            Json(AgentOwnersResponse {
                agents: agents.into_iter().map(AgentOwnerResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(failure_response(failure, correlation_id)),
    }
}

/// 逗号分隔、去重；空的、编号不对的、超过一次的上限，都当作请求不对。
fn parse_agent_ids(value: &str) -> Option<Vec<AgentId>> {
    let mut ids: Vec<AgentId> = Vec::new();
    for candidate in value.split(',').filter(|candidate| !candidate.is_empty()) {
        let id = parse_uuid_v7(candidate).ok().map(AgentId::from_uuid)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    (!ids.is_empty() && ids.len() <= MAX_AGENT_OWNER_CANDIDATES).then_some(ids)
}

fn failure_response(failure: RoomPeopleFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure {
        RoomPeopleFailure::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_people.forbidden",
            ErrorCategory::Authorization,
            "登录已失效，请重新登录。",
        ),
        RoomPeopleFailure::NotFound => (
            StatusCode::NOT_FOUND,
            "room_people.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在这个房间里。",
        ),
        RoomPeopleFailure::TooMany => {
            return invalid("room_people.invalid_agent_ids", correlation_id);
        }
        RoomPeopleFailure::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_people.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时查不到，请稍后再试。",
        ),
        RoomPeopleFailure::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_people.internal",
            ErrorCategory::Transient,
            "服务器内部错误。",
        ),
    };
    ApiError::new(status, code, category, message, correlation_id).into_response()
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    ApiError::invalid_request(code, correlation_id).into_response()
}

#[cfg(test)]
mod tests;
