//! 这台设备的账号能进哪些房间。CLI 与 MCP 用它按名字解析房间，人就不必再从应用里复制房间参数。
//! 公开房间的 `slug` 是它的分享码（`/r/<分享码>`），主人贴过来的分享链接、房间号也按这份名单认。
//!
//! 也能翻发现页的公开房间（[`ControlPlaneRoomDirectoryGateway::list_public`]）：本机 Agent 跟着主人
//! 进，账号还没加入的那几间要请主人先打开分享链接加入。
//!
//! 名单里也有子房间（`specs/redesign/sub-rooms.md` 第 2g 步），排在父房间后面。按名字找子房间要写成
//! `父房间名/子房间名`：子房间常和别处的房间同名，只写名字找的是整间的房间。

use agent_room_application::ports::PortFuture;
use agent_room_domain::{
    ids::RoomCatalogId,
    join_codes::parse_room_number,
    room_listing::{RoomJoinMode, RoomShareCode},
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
    /// 下面三项只有子房间才有：它在哪间房下面、分类的名字（“未分类”的没有）、哪种门（`Open` 是
    /// 没有门）。有门、账号又不在里面的也列出来，Agent 照着进会被告知要主人先申请。
    pub parent: Option<ParentRoom>,
    pub category: Option<String>,
    pub access: Option<RoomJoinMode>,
}

/// 子房间在哪间房下面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRoom {
    pub catalog_id: RoomCatalogId,
    pub name: String,
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

/// 发现页上的一间公开房间，不登录就看得到的那几样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomListing {
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    pub owner_name: String,
    pub join_mode: RoomJoinMode,
    pub end_to_end_encrypted: bool,
    pub member_count: u32,
    pub agent_count: u32,
    pub last_message_at_unix_ms: Option<i64>,
    pub share_code: RoomShareCode,
    pub share_url: String,
}

/// 一页公开房间；还有下一页时带着翻页的记号，原样交回去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomListingPage {
    pub rooms: Vec<PublicRoomListing>,
    pub next_cursor: Option<String>,
}

pub trait ControlPlaneRoomDirectoryGateway: Send + Sync {
    fn list_accessible(&self) -> PortFuture<'_, RoomDirectoryResult<Vec<AccessibleRoom>>>;

    /// 发现页的一页公开房间，最近有人说话的在前；`search` 搜名字、简介和房主的昵称。
    fn list_public<'a>(
        &'a self,
        search: Option<&'a str>,
        cursor: Option<&'a str>,
    ) -> PortFuture<'a, RoomDirectoryResult<PublicRoomListingPage>>;
}

/// 能按名字被找到的房间：目录条目和 IPC 摘要都实现它，CLI 与 MCP 共用一套解析规则。
pub trait NamedRoom {
    fn room_name(&self) -> &str;
    fn room_slug(&self) -> Option<&str>;
    fn room_catalog_id(&self) -> Option<RoomCatalogId>;
    /// 公开房间的分享码：私人房间的 `slug`（公共大厅的 `slug` 是大厅的名字，不算）。
    fn room_share_code(&self) -> Option<&str>;
    /// 子房间的父房间的名字；整间的房间没有。
    fn room_parent_name(&self) -> Option<&str>;
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

    fn room_parent_name(&self) -> Option<&str> {
        self.parent.as_ref().map(|parent| parent.name.as_str())
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

/// 按名字找房间：先精确匹配，再忽略大小写匹配；同名多间视为歧义，交给调用方列出候选。子房间要写成
/// `父房间名/子房间名`（斜杠两边的空白不算）；只写名字找的是整间的房间。
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
        .filter(|room| named(*room, wanted, |left, right| left == right))
        .collect();
    let candidates = if exact.is_empty() {
        rooms
            .iter()
            .filter(|room| named(*room, wanted, str::eq_ignore_ascii_case))
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

/// 整间的房间比名字和别名；子房间比 `父房间名/子房间名`。子房间的名字里不能有斜杠，父房间的可以，
/// 所以从最后一个斜杠断开。
fn named<R: NamedRoom>(room: &R, wanted: &str, same: fn(&str, &str) -> bool) -> bool {
    match room.room_parent_name() {
        None => {
            same(room.room_name(), wanted)
                || room.room_slug().is_some_and(|slug| same(slug, wanted))
        }
        Some(parent) => wanted.rsplit_once('/').is_some_and(|(left, right)| {
            same(parent, left.trim()) && same(room.room_name(), right.trim())
        }),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        AccessibleRoom, AccessibleRoomKind, ParentRoom, resolve_room_by_name, resolve_room_link,
    };
    use agent_room_domain::{ids::RoomCatalogId, room_listing::RoomJoinMode};

    fn room(name: &str, slug: Option<&str>) -> AccessibleRoom {
        AccessibleRoom {
            kind: AccessibleRoomKind::PublicLobby,
            catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
            matrix_room_id: None,
            name: name.to_owned(),
            slug: slug.map(str::to_owned),
            membership: None,
            parent: None,
            category: None,
            access: None,
        }
    }

    fn sub_room(parent: &AccessibleRoom, name: &str) -> AccessibleRoom {
        AccessibleRoom {
            kind: AccessibleRoomKind::PrivateRoom,
            parent: Some(ParentRoom {
                catalog_id: parent.catalog_id,
                name: parent.name.clone(),
            }),
            access: Some(RoomJoinMode::Open),
            ..room(name, None)
        }
    }

    #[test]
    fn 子房间写成父房间名斜杠子房间名_只写名字找的是整间的房间() {
        let project = room("Project", None);
        let weekly = room("Weekly", None);
        let general = room("General", None);
        let plans = room("Q3/Q4 plans", None);
        let rooms = vec![
            project.clone(),
            sub_room(&project, "General"),
            plans.clone(),
            sub_room(&plans, "Budget"),
            weekly.clone(),
            sub_room(&weekly, "General"),
            general,
        ];

        let found = resolve_room_by_name(&rooms, "General")
            .expect("只有一间整的叫这个")
            .expect("找得到");
        assert!(found.parent.is_none(), "只写名字不找子房间");
        let found = resolve_room_by_name(&rooms, "Project/General")
            .expect("唯一")
            .expect("找得到");
        assert_eq!(
            found.parent.as_ref().map(|parent| parent.catalog_id),
            Some(project.catalog_id)
        );
        let found = resolve_room_by_name(&rooms, " weekly / general ")
            .expect("忽略大小写和斜杠两边的空白")
            .expect("找得到");
        assert_eq!(
            found.parent.as_ref().map(|parent| parent.catalog_id),
            Some(weekly.catalog_id)
        );
        let found = resolve_room_by_name(&rooms, "Q3/Q4 plans/Budget")
            .expect("父房间名字里的斜杠也认")
            .expect("找得到");
        assert_eq!(found.name, "Budget");
        let found = resolve_room_by_name(&rooms, "Q3/Q4 plans")
            .expect("整间的房间名字里有斜杠")
            .expect("找得到");
        assert!(found.parent.is_none());
        assert!(
            resolve_room_by_name(&rooms, "Budget")
                .expect("只写子房间的名字")
                .is_none()
        );
        let by_number = resolve_room_link(&rooms, &rooms[1].catalog_id.to_string())
            .expect("是房间号")
            .expect("子房间也能拿房间号进");
        assert_eq!(by_number.name, "General");
        assert!(by_number.parent.is_some());
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
