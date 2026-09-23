//! 这台设备的账号能进哪些房间。CLI 与 MCP 用它按名字解析房间，人就不必再从应用里复制房间参数。

use agent_room_application::ports::PortFuture;
use agent_room_domain::{ids::RoomCatalogId, rooms::MatrixRoomReference};

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
}

impl NamedRoom for AccessibleRoom {
    fn room_name(&self) -> &str {
        &self.name
    }

    fn room_slug(&self) -> Option<&str> {
        self.slug.as_deref()
    }
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

    use super::{AccessibleRoom, AccessibleRoomKind, resolve_room_by_name};
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
}
