//! 向控制面询问这台设备的账号能进哪些房间（`GET /lobbies/accessible`，设备签名），也翻发现页的公开
//! 房间（`GET /public-rooms`，不要登录，不签名）。

use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::{
    room_directory::{
        AccessibleRoom, AccessibleRoomKind, AccessibleRoomMembership,
        ControlPlaneRoomDirectoryGateway, PublicRoomListing, PublicRoomListingPage,
        RoomDirectoryFailure, RoomDirectoryFailureKind, RoomDirectoryResult,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::RoomCatalogId,
    room_listing::{RoomJoinMode, RoomShareCode},
    rooms::MatrixRoomReference,
};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

const ACCESSIBLE_ROOMS_TARGET: &str = "/lobbies/accessible";
const PUBLIC_ROOMS_TARGET: &str = "/public-rooms";

pub struct ReqwestControlPlaneRoomDirectoryGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneRoomDirectoryGateway {
    /// 创建按设备签名读取可进房间目录的 HTTP 网关。
    ///
    /// # Errors
    ///
    /// 控制面地址、明文传输边界、超时或 HTTP 客户端配置无效时返回错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        Ok(Self {
            client,
            base_url,
            authorizer,
        })
    }

    async fn list_internal(&self) -> RoomDirectoryResult<Vec<AccessibleRoom>> {
        let authorized = self
            .authorizer
            .authorize("GET", ACCESSIBLE_ROOMS_TARGET, "")
            .await
            .map_err(map_session_failure)?;
        let request_url = self
            .base_url
            .join(ACCESSIBLE_ROOMS_TARGET.trim_start_matches('/'))
            .map_err(|_| failure(RoomDirectoryFailureKind::Internal))?;
        let request = signed_request_headers(
            self.client.get(request_url),
            &authorized,
            "GET",
            ACCESSIBLE_ROOMS_TARGET,
        )
        .map_err(|()| failure(RoomDirectoryFailureKind::Internal))?;
        let response = request.send().await.map_err(|error| {
            if error.is_connect() || error.is_timeout() {
                failure(RoomDirectoryFailureKind::ControlPlaneUnavailable)
            } else {
                failure(RoomDirectoryFailureKind::Internal)
            }
        })?;
        let status = response.status();
        let body = read_limited_response_body(response)
            .await
            .map_err(|()| failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse))?;
        if !status.is_success() {
            return Err(status_failure(status));
        }
        let decoded: AccessibleRoomsResponse = serde_json::from_slice(&body)
            .map_err(|_| failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse))?;
        decoded
            .rooms
            .into_iter()
            .map(AccessibleRoom::try_from)
            .collect()
    }
}

impl ReqwestControlPlaneRoomDirectoryGateway {
    async fn list_public_internal(
        &self,
        search: Option<&str>,
        cursor: Option<&str>,
    ) -> RoomDirectoryResult<PublicRoomListingPage> {
        let mut request_url = self
            .base_url
            .join(PUBLIC_ROOMS_TARGET.trim_start_matches('/'))
            .map_err(|_| failure(RoomDirectoryFailureKind::Internal))?;
        {
            let mut query = request_url.query_pairs_mut();
            if let Some(search) = search {
                query.append_pair("q", search);
            }
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
        }
        if request_url.query() == Some("") {
            request_url.set_query(None);
        }
        let response = self.client.get(request_url).send().await.map_err(|error| {
            if error.is_connect() || error.is_timeout() {
                failure(RoomDirectoryFailureKind::ControlPlaneUnavailable)
            } else {
                failure(RoomDirectoryFailureKind::Internal)
            }
        })?;
        let status = response.status();
        let body = read_limited_response_body(response)
            .await
            .map_err(|()| failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse))?;
        if !status.is_success() {
            return Err(status_failure(status));
        }
        let decoded: PublicRoomsResponse = serde_json::from_slice(&body)
            .map_err(|_| failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse))?;
        Ok(PublicRoomListingPage {
            rooms: decoded
                .rooms
                .into_iter()
                .map(PublicRoomListing::try_from)
                .collect::<RoomDirectoryResult<_>>()?,
            next_cursor: decoded.next_cursor,
        })
    }
}

impl ControlPlaneRoomDirectoryGateway for ReqwestControlPlaneRoomDirectoryGateway {
    fn list_accessible(&self) -> PortFuture<'_, RoomDirectoryResult<Vec<AccessibleRoom>>> {
        Box::pin(self.list_internal())
    }

    fn list_public<'a>(
        &'a self,
        search: Option<&'a str>,
        cursor: Option<&'a str>,
    ) -> PortFuture<'a, RoomDirectoryResult<PublicRoomListingPage>> {
        Box::pin(self.list_public_internal(search, cursor))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicRoomsResponse {
    rooms: Vec<PublicRoomResponse>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// 发现页回答里的一间房；登录的人才有的 `viewer` 和这里用不上的几样不读。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicRoomResponse {
    catalog_id: String,
    share_code: String,
    share_url: String,
    name: String,
    description: String,
    owner_name: String,
    member_count: u32,
    agent_count: u32,
    join_mode: String,
    end_to_end_encrypted: bool,
    #[serde(default)]
    last_message_at_unix_ms: Option<i64>,
}

impl TryFrom<PublicRoomResponse> for PublicRoomListing {
    type Error = RoomDirectoryFailure;

    fn try_from(value: PublicRoomResponse) -> Result<Self, Self::Error> {
        let invalid = || failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse);
        let catalog_id = uuid::Uuid::parse_str(&value.catalog_id)
            .ok()
            .filter(|id| id.get_version() == Some(uuid::Version::SortRand))
            .map(RoomCatalogId::from_uuid)
            .ok_or_else(invalid)?;
        let share_code = RoomShareCode::parse(&value.share_code).map_err(|_| invalid())?;
        let join_mode = RoomJoinMode::try_from(value.join_mode.as_str()).map_err(|_| invalid())?;
        let share_url = Url::parse(&value.share_url)
            .ok()
            .filter(|url| matches!(url.scheme(), "https" | "http"))
            .ok_or_else(invalid)?;
        if value.name.trim().is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            catalog_id,
            name: value.name,
            description: value.description,
            owner_name: value.owner_name,
            join_mode,
            end_to_end_encrypted: value.end_to_end_encrypted,
            member_count: value.member_count,
            agent_count: value.agent_count,
            last_message_at_unix_ms: value.last_message_at_unix_ms,
            share_code,
            share_url: share_url.into(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessibleRoomsResponse {
    rooms: Vec<AccessibleRoomResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessibleRoomResponse {
    kind: String,
    catalog_id: String,
    #[serde(default)]
    matrix_room_id: Option<String>,
    name: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    membership: Option<String>,
}

impl TryFrom<AccessibleRoomResponse> for AccessibleRoom {
    type Error = RoomDirectoryFailure;

    fn try_from(value: AccessibleRoomResponse) -> Result<Self, Self::Error> {
        let invalid = || failure(RoomDirectoryFailureKind::InvalidControlPlaneResponse);
        let kind = match value.kind.as_str() {
            "public_lobby" => AccessibleRoomKind::PublicLobby,
            "private_room" => AccessibleRoomKind::PrivateRoom,
            _ => return Err(invalid()),
        };
        let catalog_id = uuid::Uuid::parse_str(&value.catalog_id)
            .ok()
            .filter(|id| id.get_version() == Some(uuid::Version::SortRand))
            .map(RoomCatalogId::from_uuid)
            .ok_or_else(invalid)?;
        let matrix_room_id = value
            .matrix_room_id
            .map(MatrixRoomReference::new)
            .transpose()
            .map_err(|_| invalid())?;
        let membership = match value.membership.as_deref() {
            None => None,
            Some("invited") => Some(AccessibleRoomMembership::Invited),
            Some("joined") => Some(AccessibleRoomMembership::Joined),
            Some(_) => return Err(invalid()),
        };
        if value.name.trim().is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            kind,
            catalog_id,
            matrix_room_id,
            name: value.name,
            slug: value.slug,
            membership,
        })
    }
}

const fn failure(kind: RoomDirectoryFailureKind) -> RoomDirectoryFailure {
    RoomDirectoryFailure::new(kind)
}

fn status_failure(status: StatusCode) -> RoomDirectoryFailure {
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => RoomDirectoryFailureKind::NotAuthorized,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => RoomDirectoryFailureKind::ControlPlaneUnavailable,
        _ if status.is_server_error() => RoomDirectoryFailureKind::ControlPlaneUnavailable,
        _ => RoomDirectoryFailureKind::InvalidControlPlaneResponse,
    };
    failure(kind)
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> RoomDirectoryFailure {
    let kind = match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized => RoomDirectoryFailureKind::NotAuthorized,
        BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            RoomDirectoryFailureKind::ControlPlaneUnavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => RoomDirectoryFailureKind::Internal,
    };
    failure(kind)
}

#[cfg(test)]
mod tests {
    use super::{AccessibleRoomResponse, RoomDirectoryFailureKind};
    use agent_room_bridge_core::room_directory::{
        AccessibleRoom, AccessibleRoomKind, AccessibleRoomMembership, PublicRoomListing,
    };
    use agent_room_domain::room_listing::RoomJoinMode;

    fn response(kind: &str, membership: Option<&str>) -> AccessibleRoomResponse {
        AccessibleRoomResponse {
            kind: kind.to_owned(),
            catalog_id: "0198b601-77a1-7bb8-83eb-a8fe68c97e46".to_owned(),
            matrix_room_id: Some("!room:matrix.agent-room.test".to_owned()),
            name: "game dev".to_owned(),
            slug: None,
            membership: membership.map(str::to_owned),
        }
    }

    #[test]
    fn 控制面回答按字段校验后才变成房间() {
        let room =
            AccessibleRoom::try_from(response("private_room", Some("joined"))).expect("有效");
        assert_eq!(room.kind, AccessibleRoomKind::PrivateRoom);
        assert_eq!(room.membership, Some(AccessibleRoomMembership::Joined));
        assert_eq!(
            AccessibleRoom::try_from(response("something_else", None))
                .expect_err("未知类型")
                .kind(),
            RoomDirectoryFailureKind::InvalidControlPlaneResponse
        );
        assert_eq!(
            AccessibleRoom::try_from(response("public_lobby", Some("owner")))
                .expect_err("未知成员状态")
                .kind(),
            RoomDirectoryFailureKind::InvalidControlPlaneResponse
        );
        let mut blank = response("public_lobby", None);
        blank.name = "  ".to_owned();
        assert!(AccessibleRoom::try_from(blank).is_err());
    }

    #[test]
    fn 发现页的回答认得出_登录才有的字段不读_格式不对的整页不收() {
        let body = serde_json::json!({
            "rooms": [{
                "catalogId": "0198b601-77a1-7bb8-83eb-a8fe68c97e46",
                "shareCode": "Club01",
                "shareUrl": "https://agent-room.test/r/Club01",
                "name": "Book club",
                "description": "One book a week",
                "ownerName": "Ming",
                "memberCount": 12,
                "agentCount": 3,
                "joinMode": "invite_only",
                "endToEndEncrypted": true,
                "createdAtUnixMs": 1_000,
                "lastMessageAtUnixMs": 2_000,
                "viewer": "joined"
            }],
            "nextCursor": "24"
        });
        let decoded: super::PublicRoomsResponse = serde_json::from_value(body).unwrap();
        assert_eq!(decoded.next_cursor.as_deref(), Some("24"));
        let room = decoded.rooms.into_iter().next().unwrap();
        let listing = PublicRoomListing::try_from(room).expect("认得");
        assert_eq!(listing.join_mode, RoomJoinMode::InviteOnly);
        assert_eq!(listing.share_code.as_str(), "Club01");
        assert_eq!(listing.share_url, "https://agent-room.test/r/Club01");
        assert_eq!(listing.last_message_at_unix_ms, Some(2_000));

        for (field, value) in [
            ("joinMode", "everyone"),
            ("shareUrl", "javascript:alert(1)"),
            ("shareCode", "no"),
            ("catalogId", "not-a-uuid"),
        ] {
            let mut room = serde_json::json!({
                "catalogId": "0198b601-77a1-7bb8-83eb-a8fe68c97e46",
                "shareCode": "Club01",
                "shareUrl": "https://agent-room.test/r/Club01",
                "name": "Book club",
                "description": "",
                "ownerName": "Ming",
                "memberCount": 1,
                "agentCount": 0,
                "joinMode": "open",
                "endToEndEncrypted": false
            });
            room[field] = serde_json::Value::from(value);
            let room: super::PublicRoomResponse = serde_json::from_value(room).unwrap();
            assert_eq!(
                PublicRoomListing::try_from(room).unwrap_err().kind(),
                RoomDirectoryFailureKind::InvalidControlPlaneResponse,
                "{field}"
            );
        }
    }
}
