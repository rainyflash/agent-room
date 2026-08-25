use std::{collections::BTreeMap, fmt, sync::Arc};

use agent_room_application::ports::{
    AgentRoomMembershipFactory, DirectMatrixRoomCreation, DirectSessionMatrixProvisioner,
    MatrixCreateRoom, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation,
    MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomEncryption, MatrixRoomId, MatrixRoomKind,
    MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, PortFuture,
    PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
    PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner, RoomMembershipGateway,
    RoomProvisioningGateway,
};
use agent_room_domain::rooms::MatrixRoomReference;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, map_matrix_error,
    map_transport_error, read_limited_body,
};

/// 仅能代表一个已验证的受管 Agent 用户执行入房和退房。
#[derive(Clone)]
pub struct MatrixApplicationServiceRoomMembership {
    application_service: Arc<MatrixApplicationServiceProvisioner>,
    user_id: MatrixUserId,
}

impl MatrixApplicationServiceRoomMembership {
    pub(crate) const fn new(
        application_service: Arc<MatrixApplicationServiceProvisioner>,
        user_id: MatrixUserId,
    ) -> Self {
        Self {
            application_service,
            user_id,
        }
    }

    async fn change_membership(
        &self,
        room_id: &MatrixRoomReference,
        action: &'static str,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let mut endpoint = endpoint_with_segments(
            &self.application_service.homeserver_url,
            &["_matrix", "client", "v3", "rooms", room_id.as_str(), action],
            operation,
        )?;
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", self.user_id.as_str());
        let response = self
            .application_service
            .client
            .post(endpoint)
            .bearer_auth(self.application_service.access_token.expose())
            .json(&EmptyRequest {})
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        expect_empty_success(response, operation).await
    }
}

impl fmt::Debug for MatrixApplicationServiceRoomMembership {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixApplicationServiceRoomMembership")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

impl RoomMembershipGateway for MatrixApplicationServiceRoomMembership {
    fn join<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.change_membership(room_id, "join", MatrixOperation::Join))
    }

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.change_membership(room_id, "leave", MatrixOperation::Leave))
    }
}

impl AgentRoomMembershipFactory for MatrixApplicationServiceProvisioner {
    fn bind(&self, matrix_user_id: &MatrixUserId) -> MatrixResult<Arc<dyn RoomMembershipGateway>> {
        self.room_membership(matrix_user_id.clone())
            .map(|membership| Arc::new(membership) as Arc<dyn RoomMembershipGateway>)
    }
}

impl RoomProvisioningGateway for MatrixApplicationServiceProvisioner {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(create_room(self, request, None))
    }

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let operation = MatrixOperation::ResolveRoomAlias;
            let alias = format!("#{}:{}", alias_localpart.as_str(), self.server_name);
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &["_matrix", "client", "v3", "directory", "room", &alias],
                operation,
            )?;
            let response = self
                .client
                .get(endpoint)
                .bearer_auth(self.access_token.expose())
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            let body = expect_success_body(response, operation).await?;
            let resolved: ResolveRoomAliasResponse = decode_json(&body, operation)?;
            MatrixRoomId::new(resolved.room_id).map_err(|_| invalid_response(operation))
        })
    }

    fn attach_child<'a>(
        &'a self,
        space_id: &'a MatrixRoomId,
        child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            let operation = MatrixOperation::SendStateEvent;
            let child_server = matrix_server_name(child_id.as_str(), operation)?;
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    space_id.as_str(),
                    "state",
                    "m.space.child",
                    child_id.as_str(),
                ],
                operation,
            )?;
            let response = self
                .client
                .put(endpoint)
                .bearer_auth(self.access_token.expose())
                .json(&json!({ "via": [child_server], "suggested": true }))
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            let body = expect_success_body(response, operation).await?;
            let accepted: StateEventResponse = decode_json(&body, operation)?;
            MatrixEventId::new(accepted.event_id).map_err(|_| invalid_response(operation))
        })
    }
}

impl PrivateRoomMatrixProvisioner for MatrixApplicationServiceProvisioner {
    fn create<'a>(
        &'a self,
        creation: &'a PrivateMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(create_or_reconcile_room(
            self,
            creation.request(),
            creation.alias(),
            None,
        ))
    }
}

impl DirectSessionMatrixProvisioner for MatrixApplicationServiceProvisioner {
    fn create<'a>(
        &'a self,
        creation: &'a DirectMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            self.ensure_managed_user(creation.creator(), MatrixOperation::CreateRoom)?;
            let room_id = create_or_reconcile_room(
                self,
                creation.request(),
                creation.alias(),
                Some(creation.creator()),
            )
            .await?;
            record_direct_room(self, creation.creator(), creation.peer(), &room_id).await?;
            Ok(room_id)
        })
    }
}

impl PrivateRoomMatrixGateway for MatrixApplicationServiceProvisioner {
    fn membership<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        Box::pin(async move {
            let operation = MatrixOperation::InspectMembership;
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    room_id.as_str(),
                    "state",
                    "m.room.member",
                    user_id.as_str(),
                ],
                operation,
            )?;
            let response = self
                .client
                .get(endpoint)
                .bearer_auth(self.access_token.expose())
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            let body = expect_success_body(response, operation).await?;
            let membership: MembershipResponse = decode_json(&body, operation)?;
            decode_membership(&membership.membership, operation).map(Some)
        })
    }

    fn invite<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if matches!(
                PrivateRoomMatrixGateway::membership(self, room_id, user_id).await?,
                Some(PrivateMatrixMembership::Invited | PrivateMatrixMembership::Joined)
            ) {
                return Ok(());
            }
            send_membership_action(
                self,
                room_id,
                user_id,
                "invite",
                None,
                MatrixOperation::Invite,
            )
            .await
        })
    }

    fn kick<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if matches!(
                PrivateRoomMatrixGateway::membership(self, room_id, user_id).await?,
                None | Some(PrivateMatrixMembership::Left)
            ) {
                return Ok(());
            }
            send_membership_action(
                self,
                room_id,
                user_id,
                "kick",
                Some("Agent Room 私人房间权限已撤销"),
                MatrixOperation::Kick,
            )
            .await
        })
    }

    fn ban<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if matches!(
                PrivateRoomMatrixGateway::membership(self, room_id, user_id).await?,
                Some(PrivateMatrixMembership::Banned)
            ) {
                return Ok(());
            }
            send_membership_action(
                self,
                room_id,
                user_id,
                "ban",
                Some("Agent Room 私人房间成员已被封禁"),
                MatrixOperation::Ban,
            )
            .await
        })
    }

    fn set_speaking<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let operation = MatrixOperation::UpdatePowerLevels;
            let mut content = read_power_levels(self, room_id, operation).await?;
            apply_active_private_policy(
                &mut content,
                &[PrivateMatrixSpeakingAssignment::new(
                    user_id.clone(),
                    allowed,
                )],
                operation,
            )?;
            write_power_levels(self, room_id, &content, operation).await
        })
    }

    fn set_speaking_batch<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if assignments.is_empty() {
                return Ok(());
            }
            let operation = MatrixOperation::UpdatePowerLevels;
            let mut content = read_power_levels(self, room_id, operation).await?;
            apply_active_private_policy(&mut content, assignments, operation)?;
            write_power_levels(self, room_id, &content, operation).await
        })
    }

    fn archive<'a>(&'a self, room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let operation = MatrixOperation::ArchiveRoom;
            let mut content = read_power_levels(self, room_id, operation).await?;
            apply_archived_private_policy(&mut content);
            write_power_levels(self, room_id, &content, operation).await
        })
    }
}

#[derive(Serialize)]
struct EmptyRequest {}

async fn create_room(
    provisioner: &MatrixApplicationServiceProvisioner,
    request: &MatrixCreateRoom,
    asserted_user: Option<&MatrixUserId>,
) -> MatrixResult<MatrixRoomId> {
    let operation = MatrixOperation::CreateRoom;
    let mut endpoint = provisioner.endpoint("_matrix/client/v3/createRoom", operation)?;
    if let Some(user_id) = asserted_user {
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", user_id.as_str());
    }
    let response = provisioner
        .client
        .post(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(&CreateRoomRequest::from(request))
        .send()
        .await
        .map_err(|error| map_create_transport_error(operation, &error))?;
    let body = expect_success_body(response, operation).await?;
    let created: CreateRoomResponse = decode_json(&body, operation)?;
    MatrixRoomId::new(created.room_id).map_err(|_| invalid_response(operation))
}

async fn create_or_reconcile_room(
    provisioner: &MatrixApplicationServiceProvisioner,
    request: &MatrixCreateRoom,
    alias: &MatrixRoomAliasLocalpart,
    asserted_user: Option<&MatrixUserId>,
) -> MatrixResult<MatrixRoomId> {
    let failure = match create_room(provisioner, request, asserted_user).await {
        Ok(room_id) => return Ok(room_id),
        Err(failure)
            if matches!(
                failure.kind(),
                MatrixFailureKind::Conflict | MatrixFailureKind::UnknownCommit
            ) =>
        {
            failure
        }
        Err(failure) => return Err(failure),
    };

    match RoomProvisioningGateway::resolve_room_alias(provisioner, alias).await {
        Ok(room_id) => Ok(room_id),
        Err(_) if failure.kind() == MatrixFailureKind::UnknownCommit => Err(failure),
        Err(resolve_failure) => Err(resolve_failure),
    }
}

async fn record_direct_room(
    provisioner: &MatrixApplicationServiceProvisioner,
    owner: &MatrixUserId,
    peer: &MatrixUserId,
    room_id: &MatrixRoomId,
) -> MatrixResult<()> {
    let mut account_data = read_direct_account_data(provisioner, owner).await?;
    let rooms = account_data.entry(peer.as_str().to_owned()).or_default();
    if !rooms.iter().any(|candidate| candidate == room_id.as_str()) {
        rooms.push(room_id.as_str().to_owned());
    }
    write_direct_account_data(provisioner, owner, &account_data).await
}

async fn read_direct_account_data(
    provisioner: &MatrixApplicationServiceProvisioner,
    owner: &MatrixUserId,
) -> MatrixResult<BTreeMap<String, Vec<String>>> {
    let operation = MatrixOperation::ReadAccountData;
    let endpoint = account_data_endpoint(provisioner, owner, operation)?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    match expect_success_body(response, operation).await {
        Ok(body) => decode_json(&body, operation),
        Err(failure) if failure.kind() == MatrixFailureKind::NotFound => Ok(BTreeMap::new()),
        Err(failure) => Err(failure),
    }
}

async fn write_direct_account_data(
    provisioner: &MatrixApplicationServiceProvisioner,
    owner: &MatrixUserId,
    account_data: &BTreeMap<String, Vec<String>>,
) -> MatrixResult<()> {
    let operation = MatrixOperation::SetAccountData;
    let endpoint = account_data_endpoint(provisioner, owner, operation)?;
    let response = provisioner
        .client
        .put(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(account_data)
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

fn account_data_endpoint(
    provisioner: &MatrixApplicationServiceProvisioner,
    owner: &MatrixUserId,
    operation: MatrixOperation,
) -> MatrixResult<Url> {
    let mut endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "user",
            owner.as_str(),
            "account_data",
            "m.direct",
        ],
        operation,
    )?;
    endpoint
        .query_pairs_mut()
        .append_pair("user_id", owner.as_str());
    Ok(endpoint)
}

#[derive(Serialize)]
struct CreateRoomRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<&'a str>,
    visibility: &'static str,
    preset: &'static str,
    is_direct: bool,
    invite: Vec<&'a str>,
    creation_content: Value,
    initial_state: Vec<InitialStateEventRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_alias_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    power_level_content_override: Option<PowerLevelContentOverride<'a>>,
}

#[derive(Serialize)]
struct InitialStateEventRequest {
    #[serde(rename = "type")]
    event_type: &'static str,
    state_key: &'static str,
    content: Value,
}

#[derive(Serialize)]
struct PowerLevelContentOverride<'a> {
    events: BTreeMap<&'a str, i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    events_default: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_default: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    users_default: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invite: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kick: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ban: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redact: Option<i64>,
}

impl<'a> From<&'a MatrixCreateRoom> for CreateRoomRequest<'a> {
    fn from(request: &'a MatrixCreateRoom) -> Self {
        let visibility = match request.visibility() {
            MatrixRoomVisibility::Private => "private",
            MatrixRoomVisibility::Public => "public",
        };
        let preset = match request.preset() {
            MatrixRoomPreset::PrivateChat => "private_chat",
            MatrixRoomPreset::PublicChat => "public_chat",
            MatrixRoomPreset::TrustedPrivateChat => "trusted_private_chat",
        };
        let creation_content = match request.kind() {
            MatrixRoomKind::Conversation => json!({}),
            MatrixRoomKind::Space => json!({ "type": "m.space" }),
        };
        let member_writable_events = request.member_writable_state_event_types();
        let initial_state = match request.encryption() {
            MatrixRoomEncryption::Unencrypted => Vec::new(),
            MatrixRoomEncryption::EndToEnd => vec![InitialStateEventRequest {
                event_type: "m.room.encryption",
                state_key: "",
                content: json!({
                    "algorithm": "m.megolm.v1.aes-sha2",
                    "rotation_period_ms": 604_800_000_u64,
                    "rotation_period_msgs": 100_u64,
                }),
            }],
        };
        let managed_private = request.power_profile() == MatrixRoomPowerProfile::ManagedPrivate;
        let power_level_content_override = (managed_private || !member_writable_events.is_empty())
            .then(|| PowerLevelContentOverride {
                events: member_writable_events
                    .iter()
                    .map(|event_type| (event_type.as_str(), 0))
                    .collect(),
                events_default: managed_private.then_some(PRIVATE_SPEAKER_POWER_LEVEL),
                state_default: managed_private.then_some(PRIVATE_ADMIN_POWER_LEVEL),
                users_default: managed_private.then_some(PRIVATE_VIEWER_POWER_LEVEL),
                invite: managed_private.then_some(PRIVATE_ADMIN_POWER_LEVEL),
                kick: managed_private.then_some(PRIVATE_ADMIN_POWER_LEVEL),
                ban: managed_private.then_some(PRIVATE_ADMIN_POWER_LEVEL),
                redact: managed_private.then_some(PRIVATE_ADMIN_POWER_LEVEL),
            });
        Self {
            name: request.name(),
            topic: request.topic(),
            visibility,
            preset,
            is_direct: request.direct(),
            invite: request.invite().iter().map(MatrixUserId::as_str).collect(),
            creation_content,
            initial_state,
            room_alias_name: request
                .alias_localpart()
                .map(MatrixRoomAliasLocalpart::as_str),
            power_level_content_override,
        }
    }
}

#[derive(Deserialize)]
struct CreateRoomResponse {
    room_id: String,
}

#[derive(Deserialize)]
struct ResolveRoomAliasResponse {
    room_id: String,
}

#[derive(Deserialize)]
struct StateEventResponse {
    event_id: String,
}

#[derive(Deserialize)]
struct MembershipResponse {
    membership: String,
}

#[derive(Serialize)]
struct MembershipMutationRequest<'a> {
    user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
}

const PRIVATE_VIEWER_POWER_LEVEL: i64 = 0;
const PRIVATE_SPEAKER_POWER_LEVEL: i64 = 10;
const PRIVATE_ADMIN_POWER_LEVEL: i64 = 100;

async fn send_membership_action(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    user_id: &MatrixUserId,
    action: &'static str,
    reason: Option<&'static str>,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &["_matrix", "client", "v3", "rooms", room_id.as_str(), action],
        operation,
    )?;
    let response = provisioner
        .client
        .post(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(&MembershipMutationRequest {
            user_id: user_id.as_str(),
            reason,
        })
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

async fn read_power_levels(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    operation: MatrixOperation,
) -> MatrixResult<Map<String, Value>> {
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "state",
            "m.room.power_levels",
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    let body = expect_success_body(response, operation).await?;
    let value: Value = decode_json(&body, operation)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| invalid_response(operation))
}

async fn write_power_levels(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    content: &Map<String, Value>,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "state",
            "m.room.power_levels",
            "",
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .put(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(content)
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

fn apply_active_private_policy(
    content: &mut Map<String, Value>,
    assignments: &[PrivateMatrixSpeakingAssignment],
    operation: MatrixOperation,
) -> MatrixResult<()> {
    set_private_policy_thresholds(content, PRIVATE_SPEAKER_POWER_LEVEL);
    if assignments.is_empty() {
        return Ok(());
    }
    let users = content
        .entry("users".to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| invalid_response(operation))?;
    for assignment in assignments {
        users.insert(
            assignment.user_id().as_str().to_owned(),
            Value::from(if assignment.allowed() {
                PRIVATE_SPEAKER_POWER_LEVEL
            } else {
                PRIVATE_VIEWER_POWER_LEVEL
            }),
        );
    }
    Ok(())
}

fn apply_archived_private_policy(content: &mut Map<String, Value>) {
    set_private_policy_thresholds(content, PRIVATE_ADMIN_POWER_LEVEL);
}

fn set_private_policy_thresholds(content: &mut Map<String, Value>, events_default: i64) {
    for (key, value) in [
        ("events_default", events_default),
        ("state_default", PRIVATE_ADMIN_POWER_LEVEL),
        ("users_default", PRIVATE_VIEWER_POWER_LEVEL),
        ("invite", PRIVATE_ADMIN_POWER_LEVEL),
        ("kick", PRIVATE_ADMIN_POWER_LEVEL),
        ("ban", PRIVATE_ADMIN_POWER_LEVEL),
        ("redact", PRIVATE_ADMIN_POWER_LEVEL),
    ] {
        content.insert(key.to_owned(), Value::from(value));
    }
}

fn decode_membership(
    membership: &str,
    operation: MatrixOperation,
) -> MatrixResult<PrivateMatrixMembership> {
    match membership {
        "invite" => Ok(PrivateMatrixMembership::Invited),
        "join" => Ok(PrivateMatrixMembership::Joined),
        "leave" => Ok(PrivateMatrixMembership::Left),
        "ban" => Ok(PrivateMatrixMembership::Banned),
        "knock" => Ok(PrivateMatrixMembership::Knocked),
        _ => Err(invalid_response(operation)),
    }
}

pub(crate) async fn expect_empty_success(
    response: reqwest::Response,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    expect_success_body(response, operation).await.map(|_| ())
}

async fn expect_success_body(
    response: reqwest::Response,
    operation: MatrixOperation,
) -> MatrixResult<Vec<u8>> {
    let status = response.status();
    let body = read_limited_body(response, operation).await?;
    if status.is_success() {
        return Ok(body);
    }
    let error = decode_matrix_error(&body, operation)?;
    Err(map_matrix_error(operation, status, &error))
}

fn map_create_transport_error(operation: MatrixOperation, error: &reqwest::Error) -> MatrixFailure {
    if error.is_connect() {
        return MatrixFailure::new(operation, MatrixFailureKind::DependencyUnavailable);
    }
    MatrixFailure::new(operation, MatrixFailureKind::UnknownCommit)
}

pub(crate) fn endpoint_with_segments(
    base: &Url,
    segments: &[&str],
    operation: MatrixOperation,
) -> MatrixResult<Url> {
    let mut endpoint = base.clone();
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let mut path = endpoint
        .path_segments_mut()
        .map_err(|()| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?;
    path.clear();
    path.extend(segments);
    drop(path);
    Ok(endpoint)
}

fn matrix_server_name(room_id: &str, operation: MatrixOperation) -> MatrixResult<&str> {
    room_id
        .strip_prefix('!')
        .and_then(|value| value.rsplit_once(':'))
        .map(|(_, server_name)| server_name)
        .filter(|server_name| !server_name.is_empty())
        .ok_or_else(|| invalid_response(operation))
}

const fn invalid_response(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        DirectMatrixRoomCreation, DirectSessionMatrixProvisioner, MatrixCreateRoom,
        MatrixOperation, MatrixRoomAliasLocalpart, MatrixRoomId, MatrixRoomKind,
        MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId,
        PrivateMatrixMembership, PrivateMatrixSpeakingAssignment, PrivateRoomMatrixGateway,
        RoomMembershipGateway, RoomProvisioningGateway, SecretValue,
    };
    use agent_room_domain::rooms::MatrixRoomReference;
    use axum::{
        Json, Router,
        extract::{Path, Query},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post, put},
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, task::JoinHandle};

    use super::super::{
        MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
    };
    use super::{
        CreateRoomRequest, apply_active_private_policy, apply_archived_private_policy,
        decode_membership,
    };

    #[tokio::test]
    async fn 受管_agent_通过身份断言加入和退出房间() {
        let server = TestServer::start().await;
        let provisioner = Arc::new(provisioner(&server.url));
        let user_id = MatrixUserId::new(
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
        )
        .expect("用户标识有效");
        let membership = provisioner
            .room_membership(user_id.clone())
            .expect("受管用户可绑定成员能力");
        let room =
            MatrixRoomReference::new("!lobby:matrix.agent-room.localhost").expect("房间标识有效");

        membership.join(&room).await.expect("加入应成功");
        membership.leave(&room).await.expect("退出应成功");

        assert_eq!(server.calls().await, vec!["join", "leave"]);
        assert_eq!(
            server.asserted_users().await,
            vec![user_id.clone(), user_id]
        );
    }

    #[test]
    fn 普通_matrix_用户不能绑定_application_service_成员能力() {
        let provisioner = Arc::new(provisioner("http://127.0.0.1:9"));
        let ordinary =
            MatrixUserId::new("@ordinary:matrix.agent-room.localhost").expect("用户标识有效");

        let failure = provisioner
            .room_membership(ordinary)
            .expect_err("必须拒绝普通用户");

        assert_eq!(failure.operation(), MatrixOperation::Join);
    }

    #[test]
    fn 建房请求只下放明确登记的状态事件权限() {
        let request = MatrixCreateRoom::new(
            Some("Lobby".to_owned()),
            None,
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效")
        .with_member_writable_state_event_type(
            agent_room_application::ports::MatrixEventType::new(
                "io.github.rainyflash.agentroom.agent.status.v1",
            )
            .expect("事件类型有效"),
        );

        let body =
            serde_json::to_value(CreateRoomRequest::from(&request)).expect("建房请求可序列化");

        assert_eq!(
            body["power_level_content_override"]["events"]["io.github.rainyflash.agentroom.agent.status.v1"],
            0
        );
        assert!(body["power_level_content_override"]["events"]["m.room.power_levels"].is_null());
    }

    #[test]
    fn 私人房间建房请求使用查看者与发言者分离的硬边界() {
        let request = MatrixCreateRoom::new(
            Some("Private project".to_owned()),
            None,
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::PrivateChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效")
        .with_end_to_end_encryption()
        .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);

        let body =
            serde_json::to_value(CreateRoomRequest::from(&request)).expect("建房请求可序列化");
        let levels = &body["power_level_content_override"];

        assert_eq!(levels["users_default"], 0);
        assert_eq!(levels["events_default"], 10);
        assert_eq!(levels["state_default"], 100);
        assert_eq!(levels["invite"], 100);
        assert_eq!(levels["kick"], 100);
        assert_eq!(levels["ban"], 100);
        assert_eq!(levels["redact"], 100);
        assert!(levels["users"].is_null(), "不得覆盖 Matrix 创建者映射");
        assert_eq!(body["initial_state"][0]["type"], "m.room.encryption");
        assert_eq!(
            body["initial_state"][0]["content"]["algorithm"],
            "m.megolm.v1.aes-sha2"
        );
    }

    #[test]
    fn 发言与归档策略保留未知_matrix_字段且不会授予管理权() {
        let user = MatrixUserId::new("@member:matrix.test").expect("用户标识有效");
        let mut content = json!({
            "users": { "@service:matrix.test": 100 },
            "notifications": { "room": 50 },
            "custom_extension": { "keep": true }
        })
        .as_object()
        .expect("对象有效")
        .clone();

        apply_active_private_policy(
            &mut content,
            &[PrivateMatrixSpeakingAssignment::new(user.clone(), true)],
            MatrixOperation::UpdatePowerLevels,
        )
        .expect("发言策略有效");
        assert_eq!(content["users"][user.as_str()], 10);
        assert_eq!(content["users"]["@service:matrix.test"], 100);
        assert_eq!(content["custom_extension"]["keep"], true);
        assert_eq!(content["events_default"], 10);
        assert_eq!(content["invite"], 100);

        apply_archived_private_policy(&mut content);
        assert_eq!(content["events_default"], 100);
        assert_eq!(content["custom_extension"]["keep"], true);
    }

    #[test]
    fn matrix_成员状态只接受协议定义值() {
        assert_eq!(
            decode_membership("join", MatrixOperation::InspectMembership).expect("状态有效"),
            PrivateMatrixMembership::Joined
        );
        assert_eq!(
            decode_membership("ban", MatrixOperation::InspectMembership).expect("状态有效"),
            PrivateMatrixMembership::Banned
        );
        assert!(decode_membership("owner", MatrixOperation::InspectMembership).is_err());
    }

    #[tokio::test]
    async fn 私人房间成员与权限操作走真实_matrix_端点() {
        let server = PrivateRoomTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let room = MatrixRoomId::new("!private:matrix.agent-room.localhost").expect("房间有效");
        let member = MatrixUserId::new("@member:matrix.agent-room.localhost").expect("成员有效");

        assert_eq!(
            provisioner
                .membership(&room, &member)
                .await
                .expect("可读成员状态"),
            Some(PrivateMatrixMembership::Joined)
        );
        provisioner.invite(&room, &member).await.expect("可邀请");
        provisioner
            .set_speaking_batch(
                &room,
                &[PrivateMatrixSpeakingAssignment::new(member.clone(), true)],
            )
            .await
            .expect("可授予发言硬边界");
        provisioner.kick(&room, &member).await.expect("可移除");
        provisioner.ban(&room, &member).await.expect("可封禁");
        provisioner.archive(&room).await.expect("可归档");

        assert_eq!(
            server.actions().await,
            vec![
                "membership",
                "membership",
                "read-power",
                "write-power",
                "membership",
                "kick",
                "membership",
                "ban",
                "read-power",
                "write-power"
            ]
        );
        let writes = server.power_level_writes().await;
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0]["users"][member.as_str()], 10);
        assert_eq!(writes[0]["custom_extension"]["keep"], true);
        assert_eq!(writes[1]["events_default"], 100);
    }

    #[tokio::test]
    async fn application_service_可创建解析并挂载大厅房间() {
        let server = TestServer::start().await;
        let provisioner = provisioner(&server.url);
        let alias = MatrixRoomAliasLocalpart::new("agent-room-space-general").expect("别名有效");
        let request = MatrixCreateRoom::new(
            Some("General".to_owned()),
            Some("Public lobby".to_owned()),
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效")
        .with_kind(MatrixRoomKind::Space)
        .with_alias_localpart(alias.clone());

        let created = provisioner.create_room(&request).await.expect("建房成功");
        let resolved = provisioner
            .resolve_room_alias(&alias)
            .await
            .expect("别名解析成功");
        let child =
            MatrixRoomId::new("!child:matrix.agent-room.localhost").expect("子房间标识有效");
        let event = provisioner
            .attach_child(&created, &child)
            .await
            .expect("挂载成功");

        assert_eq!(created.as_str(), "!space:matrix.agent-room.localhost");
        assert_eq!(resolved, created);
        assert_eq!(event.as_str(), "$space-child-event");
        assert_eq!(server.calls().await, vec!["create", "resolve", "attach"]);
    }

    #[tokio::test]
    async fn 直接会话由受管_agent_创建并幂等写入_m_direct() {
        let server = DirectRoomTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let creator = MatrixUserId::new(
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
        )
        .expect("受管 Agent 标识有效");
        let peer =
            MatrixUserId::new("@principal:matrix.agent-room.localhost").expect("主体标识有效");
        let alias =
            MatrixRoomAliasLocalpart::new("agent-room-direct-session").expect("直接会话别名有效");
        let request = MatrixCreateRoom::new(
            None,
            None,
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::TrustedPrivateChat,
            true,
            vec![peer.clone()],
        )
        .expect("直接建房请求有效")
        .with_end_to_end_encryption()
        .with_alias_localpart(alias.clone());
        let creation = DirectMatrixRoomCreation::new(request, alias, creator.clone(), peer.clone())
            .expect("直接会话约束有效");

        let room_id = DirectSessionMatrixProvisioner::create(&provisioner, &creation)
            .await
            .expect("别名冲突后应对账并同步账户数据");

        assert_eq!(room_id.as_str(), "!direct:matrix.agent-room.localhost");
        assert_eq!(
            server.calls().await,
            vec![
                "create-direct",
                "resolve-direct",
                "read-direct",
                "write-direct"
            ]
        );
        assert_eq!(
            server.asserted_users().await,
            vec![creator.clone(), creator.clone(), creator]
        );
        let writes = server.account_data_writes().await;
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0]["@existing:matrix.agent-room.localhost"],
            json!(["!existing:matrix.agent-room.localhost"])
        );
        assert_eq!(
            writes[0][peer.as_str()],
            json!(["!direct:matrix.agent-room.localhost"]),
            "重复对账不能向 m.direct 追加重复房间"
        );
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        let configuration = MatrixApplicationServiceConfiguration::new(
            url,
            "matrix.agent-room.localhost",
            SecretValue::new("application-service-secret").expect("密钥有效"),
            Duration::from_secs(2),
        )
        .expect("配置有效");
        MatrixApplicationServiceProvisioner::new(configuration).expect("适配器有效")
    }

    struct TestServer {
        url: String,
        state: Arc<TestState>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct TestState {
        calls: tokio::sync::Mutex<Vec<&'static str>>,
        asserted_users: tokio::sync::Mutex<Vec<MatrixUserId>>,
    }

    impl TestServer {
        async fn start() -> Self {
            let state = Arc::new(TestState::default());
            let app = Router::new()
                .route("/_matrix/client/v3/createRoom", post(create_room))
                .route(
                    "/_matrix/client/v3/directory/room/{alias}",
                    get(resolve_room),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.space.child/{child}",
                    put(attach_child),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/{action}",
                    post(change_membership),
                )
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务可运行");
            });
            Self {
                url: format!("http://{address}"),
                state,
                task,
            }
        }

        async fn calls(&self) -> Vec<&'static str> {
            self.state.calls.lock().await.clone()
        }

        async fn asserted_users(&self) -> Vec<MatrixUserId> {
            self.state.asserted_users.lock().await.clone()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct DirectRoomTestServer {
        url: String,
        state: Arc<DirectRoomTestState>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct DirectRoomTestState {
        calls: tokio::sync::Mutex<Vec<&'static str>>,
        asserted_users: tokio::sync::Mutex<Vec<MatrixUserId>>,
        account_data_writes: tokio::sync::Mutex<Vec<Value>>,
    }

    impl DirectRoomTestServer {
        async fn start() -> Self {
            let state = Arc::new(DirectRoomTestState::default());
            let app = Router::new()
                .route("/_matrix/client/v3/createRoom", post(create_direct_room))
                .route(
                    "/_matrix/client/v3/directory/room/{alias}",
                    get(resolve_direct_room),
                )
                .route(
                    "/_matrix/client/v3/user/{user}/account_data/m.direct",
                    get(read_direct_account_data).put(write_direct_account_data),
                )
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务可运行");
            });
            Self {
                url: format!("http://{address}"),
                state,
                task,
            }
        }

        async fn calls(&self) -> Vec<&'static str> {
            self.state.calls.lock().await.clone()
        }

        async fn asserted_users(&self) -> Vec<MatrixUserId> {
            self.state.asserted_users.lock().await.clone()
        }

        async fn account_data_writes(&self) -> Vec<Value> {
            self.state.account_data_writes.lock().await.clone()
        }
    }

    impl Drop for DirectRoomTestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct PrivateRoomTestServer {
        url: String,
        state: Arc<PrivateRoomTestState>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct PrivateRoomTestState {
        actions: tokio::sync::Mutex<Vec<&'static str>>,
        power_level_writes: tokio::sync::Mutex<Vec<Value>>,
    }

    impl PrivateRoomTestServer {
        async fn start() -> Self {
            let state = Arc::new(PrivateRoomTestState::default());
            let app = Router::new()
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.member/{user}",
                    get(private_membership),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.power_levels",
                    get(read_private_power_levels),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.power_levels/",
                    put(write_private_power_levels),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/{action}",
                    post(private_membership_action),
                )
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务可运行");
            });
            Self {
                url: format!("http://{address}"),
                state,
                task,
            }
        }

        async fn actions(&self) -> Vec<&'static str> {
            self.state.actions.lock().await.clone()
        }

        async fn power_level_writes(&self) -> Vec<Value> {
            self.state.power_level_writes.lock().await.clone()
        }
    }

    impl Drop for PrivateRoomTestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn assert_authentication(headers: &HeaderMap) {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer application-service-secret")
        );
    }

    async fn create_room(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(body["room_alias_name"], "agent-room-space-general");
        assert_eq!(body["creation_content"]["type"], "m.space");
        state.calls.lock().await.push("create");
        Json(json!({ "room_id": "!space:matrix.agent-room.localhost" }))
    }

    async fn create_direct_room(
        axum::extract::State(state): axum::extract::State<Arc<DirectRoomTestState>>,
        Query(query): Query<UserQuery>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        let creator = MatrixUserId::new(query.user_id).expect("断言创建者有效");
        assert_eq!(
            creator.as_str(),
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost"
        );
        assert_eq!(body["room_alias_name"], "agent-room-direct-session");
        assert_eq!(body["visibility"], "private");
        assert_eq!(body["preset"], "trusted_private_chat");
        assert_eq!(body["is_direct"], true);
        assert_eq!(
            body["invite"],
            json!(["@principal:matrix.agent-room.localhost"])
        );
        assert!(body.get("name").is_none());
        state.calls.lock().await.push("create-direct");
        state.asserted_users.lock().await.push(creator);
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "errcode": "M_ROOM_IN_USE",
                "error": "alias already exists"
            })),
        )
            .into_response()
    }

    async fn resolve_direct_room(
        axum::extract::State(state): axum::extract::State<Arc<DirectRoomTestState>>,
        Path(alias): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(
            alias,
            "#agent-room-direct-session:matrix.agent-room.localhost"
        );
        state.calls.lock().await.push("resolve-direct");
        Json(json!({ "room_id": "!direct:matrix.agent-room.localhost" }))
    }

    async fn read_direct_account_data(
        axum::extract::State(state): axum::extract::State<Arc<DirectRoomTestState>>,
        Path(user): Path<String>,
        Query(query): Query<UserQuery>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(user, query.user_id);
        let asserted = MatrixUserId::new(query.user_id).expect("断言账户数据用户有效");
        state.calls.lock().await.push("read-direct");
        state.asserted_users.lock().await.push(asserted);
        Json(json!({
            "@existing:matrix.agent-room.localhost": ["!existing:matrix.agent-room.localhost"],
            "@principal:matrix.agent-room.localhost": ["!direct:matrix.agent-room.localhost"]
        }))
    }

    async fn write_direct_account_data(
        axum::extract::State(state): axum::extract::State<Arc<DirectRoomTestState>>,
        Path(user): Path<String>,
        Query(query): Query<UserQuery>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(user, query.user_id);
        let asserted = MatrixUserId::new(query.user_id).expect("断言账户数据用户有效");
        state.calls.lock().await.push("write-direct");
        state.asserted_users.lock().await.push(asserted);
        state.account_data_writes.lock().await.push(body);
        Json(json!({}))
    }

    async fn resolve_room(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path(alias): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(
            alias,
            "#agent-room-space-general:matrix.agent-room.localhost"
        );
        state.calls.lock().await.push("resolve");
        Json(json!({ "room_id": "!space:matrix.agent-room.localhost" }))
    }

    async fn attach_child(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path((room, child)): Path<(String, String)>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!space:matrix.agent-room.localhost");
        assert_eq!(child, "!child:matrix.agent-room.localhost");
        assert_eq!(body["via"], json!(["matrix.agent-room.localhost"]));
        state.calls.lock().await.push("attach");
        Json(json!({ "event_id": "$space-child-event" }))
    }

    #[derive(Deserialize)]
    struct UserQuery {
        user_id: String,
    }

    async fn change_membership(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path((_room, action)): Path<(String, String)>,
        Query(query): Query<UserQuery>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        let action = match action.as_str() {
            "join" => "join",
            "leave" => "leave",
            unexpected => panic!("未知动作 {unexpected}"),
        };
        state.calls.lock().await.push(action);
        state
            .asserted_users
            .lock()
            .await
            .push(MatrixUserId::new(query.user_id).expect("断言用户有效"));
        Json(json!({}))
    }

    async fn private_membership(
        axum::extract::State(state): axum::extract::State<Arc<PrivateRoomTestState>>,
        Path((room, user)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!private:matrix.agent-room.localhost");
        assert_eq!(user, "@member:matrix.agent-room.localhost");
        state.actions.lock().await.push("membership");
        Json(json!({ "membership": "join" }))
    }

    async fn private_membership_action(
        axum::extract::State(state): axum::extract::State<Arc<PrivateRoomTestState>>,
        Path((room, action)): Path<(String, String)>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!private:matrix.agent-room.localhost");
        assert_eq!(body["user_id"], "@member:matrix.agent-room.localhost");
        let action = match action.as_str() {
            "invite" => "invite",
            "kick" => {
                assert!(
                    body["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty())
                );
                "kick"
            }
            "ban" => {
                assert!(
                    body["reason"]
                        .as_str()
                        .is_some_and(|reason| !reason.is_empty())
                );
                "ban"
            }
            unexpected => panic!("未知私人房间动作 {unexpected}"),
        };
        state.actions.lock().await.push(action);
        Json(json!({}))
    }

    async fn read_private_power_levels(
        axum::extract::State(state): axum::extract::State<Arc<PrivateRoomTestState>>,
        Path(room): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!private:matrix.agent-room.localhost");
        state.actions.lock().await.push("read-power");
        Json(json!({
            "users": { "@service:matrix.agent-room.localhost": 100 },
            "custom_extension": { "keep": true }
        }))
    }

    async fn write_private_power_levels(
        axum::extract::State(state): axum::extract::State<Arc<PrivateRoomTestState>>,
        Path(room): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!private:matrix.agent-room.localhost");
        state.actions.lock().await.push("write-power");
        state.power_level_writes.lock().await.push(body);
        Json(json!({ "event_id": "$power-level" }))
    }
}
