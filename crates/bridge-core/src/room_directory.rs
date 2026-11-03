//! 这台设备的账号能进哪些房间。CLI 与 MCP 用它按名字解析房间，人就不必再从应用里复制房间参数。
//! 公开房间的 `slug` 是它的分享码（`/r/<分享码>`），主人贴过来的分享链接、房间号也按这份名单认。

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::RoomCatalogId, join_codes::parse_room_number, room_listing::RoomShareCode,
    rooms::MatrixRoomReference,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibleRoomKind {
    PublicLobby,
    PrivateRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibleRoomMembership {
    Invited,
    Joined,
}

/// 账号能进的一个房间。公开大厅在进入时才分配实例，所以没有 Matrix 房间；
/// 私人房间带实例的 Matrix 房间与账号在其中的成员状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibleRoom {
    pub kind: AccessibleRoomKind,
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: Option<MatrixRoomReference>,
    pub name: String,
    pub slug: Option<String>,
    pub membership: Option<AccessibleRoomMembership>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomDirectoryFailureKind {
    NotAuthorized,
    ControlPlaneUnavailable,
    InvalidControlPlaneResponse,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomDirectoryFailure {
    kind: RoomDirectoryFailureKind,
}

impl RoomDirectoryFailure {
    pub const fn new(kind: RoomDirectoryFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> RoomDirectoryFailureKind {
        self.kind
    }
}

pub type RoomDirectoryResult<T> = Result<T, RoomDirectoryFailure>;

pub trait ControlPlaneRoomDirectoryGateway: Send + Sync {
    fn list_accessible(&self) -> PortFuture<'_, RoomDirectoryResult<Vec<AccessibleRoom>>>;
}

/// 能按名字被找到的房间：目录条目和 IPC 摘要都实现它，CLI 与 MCP 共用一套解析规则。
pub trait NamedRoom {
    fn room_name(&self) -> &str;
    fn room_slug(&self) -> Option<&str>;
    fn room_catalog_id(&self) -> Option<RoomCatalogId>;
    /// 公开房间的分享码：私人房间的 `slug`（公共大厅的 `slug` 是大厅的名字，不算）。
    fn room_share_code(&self) -> Option<&str>;
}

impl NamedRoom for AccessibleRoom {
    fn room_name(&self) -> &str {
        &self.name
    }

    fn room_slug(&self) -> Option<&str> {
        self.slug.as_deref()
    }

    fn room_catalog_id(&self) -> Option<RoomCatalogId> {
        Some(self.catalog_id)
    }

    fn room_share_code(&self) -> Option<&str> {
        match self.kind {
            AccessibleRoomKind::PrivateRoom => self.slug.as_deref(),
            AccessibleRoomKind::PublicLobby => None,
        }
    }
}

/// 主人贴过来的链接或房间号：公开房间的分享链接（`/r/<分享码>`、发现页的 `/explore/<分享码>`）
/// 认分享码，房间号或 `/lobby/<房间号>` 的网址认房间号。不是这两种时回 `None`，交给
/// [`resolve_room_by_name`] 按名字找；是的话回能进的房间里对得上的那间，没有就是 `Some(None)`：
/// 多半是账号还没加入这间公开房间，Agent 跟着主人进，主人得先加入。
pub fn resolve_room_link<'a, R: NamedRoom>(rooms: &'a [R], input: &str) -> Option<Option<&'a R>> {
    if let Some(code) = RoomShareCode::from_link(input) {
        return Some(
            rooms
                .iter()
                .find(|room| room.room_share_code() == Some(code.as_str())),
        );
    }
    let catalog_id = parse_room_number(input)?;
    Some(
        rooms
            .iter()
            .find(|room| room.room_catalog_id() == Some(catalog_id)),
    )
}

/// 按名字找房间：先精确匹配，再忽略大小写匹配；同名多间视为歧义，交给调用方列出候选。
///
/// # Errors
///
/// 没有匹配时返回 `None`；多于一间匹配时返回全部候选。
pub fn resolve_room_by_name<'a, R: NamedRoom>(
    rooms: &'a [R],
    name: &str,
) -> Result<Option<&'a R>, Vec<&'a R>> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    let exact: Vec<&R> = rooms
        .iter()
        .filter(|room| room.room_name() == wanted || room.room_slug() == Some(wanted))
        .collect();
    let candidates = if exact.is_empty() {
        rooms
            .iter()
            .filter(|room| {
                room.room_name().eq_ignore_ascii_case(wanted)
                    || room
                        .room_slug()
                        .is_some_and(|slug| slug.eq_ignore_ascii_case(wanted))
            })
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match candidates.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(only)),
        _ => Err(candidates),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{AccessibleRoom, AccessibleRoomKind, resolve_room_by_name, resolve_room_link};
    use agent_room_domain::ids::RoomCatalogId;

    fn room(name: &str, slug: Option<&str>) -> AccessibleRoom {
        AccessibleRoom {
            kind: AccessibleRoomKind::PublicLobby,
            catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
            matrix_room_id: None,
            name: name.to_owned(),
            slug: slug.map(str::to_owned),
            membership: None,
        }
    }

    #[test]
    fn 按名字或别名解析_精确优先_同名报歧义() {
        let rooms = vec![
            room("game dev", Some("game-dev")),
            room("Game Dev", None),
            room("ops", Some("ops")),
        ];
        assert_eq!(
            resolve_room_by_name(&rooms, "game dev")
                .expect("唯一")
                .map(|r| r.slug.as_deref()),
            Some(Some("game-dev"))
        );
        assert_eq!(
            resolve_room_by_name(&rooms, "game-dev")
                .expect("别名唯一")
                .map(|r| &r.name),
            Some(&"game dev".to_owned())
        );
        assert_eq!(
            resolve_room_by_name(&rooms, "OPS")
                .expect("忽略大小写")
                .map(|r| &r.name),
            Some(&"ops".to_owned())
        );
        assert_eq!(
            resolve_room_by_name(&rooms, "GAME DEV")
                .expect_err("两间同名只差大小写")
                .len(),
            2
        );
        assert!(
            resolve_room_by_name(&rooms, "missing")
                .expect("没有")
                .is_none()
        );
        assert!(
            resolve_room_by_name(&rooms, "  ")
                .expect("空名字")
                .is_none()
        );
    }

    #[test]
    fn 分享链接认公开房间的分享码_房间号认房间号_都不是就按名字找() {
        let mut public_room = room("读书会", Some("Abc123"));
        public_room.kind = AccessibleRoomKind::PrivateRoom;
        let mut lobby = room("Global", Some("Abc124"));
        lobby.kind = AccessibleRoomKind::PublicLobby;
        let rooms = vec![public_room.clone(), lobby.clone()];

        let found = resolve_room_link(&rooms, "https://agentroom.chat/r/Abc123")
            .expect("是分享链接")
            .expect("账号进得去");
        assert_eq!(found.name, "读书会");
        assert_eq!(
            resolve_room_link(&rooms, "https://agentroom.chat/r/Abc124").expect("是分享链接"),
            None,
            "公共大厅的 slug 不是分享码"
        );
        assert_eq!(
            resolve_room_link(&rooms, "https://agentroom.chat/r/abc123").expect("是分享链接"),
            None,
            "分享码分大小写"
        );

        let address = format!("https://agentroom.chat/lobby/{}", lobby.catalog_id);
        assert_eq!(
            resolve_room_link(&rooms, &address)
                .expect("是房间号")
                .map(|room| &room.name),
            Some(&"Global".to_owned())
        );
        let unknown = RoomCatalogId::from_uuid(Uuid::now_v7()).to_string();
        assert_eq!(resolve_room_link(&rooms, &unknown).expect("是房间号"), None);
        assert_eq!(
            resolve_room_link(&rooms, "读书会"),
            None,
            "名字交给按名字找"
        );
        assert_eq!(
            resolve_room_link(&rooms, "Abc123"),
            None,
            "光是码也按名字找"
        );
    }
}
