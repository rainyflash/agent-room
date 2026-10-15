//! 房间里的人叫什么、Agent 是谁的（`specs/room-people/design.md`）。
//!
//! 私人房间里成员的名字和 Agent 的主人，只给这个房间已加入的成员看；公共大厅里 Agent 的主人给
//! 登录了的人看。主人按服务器的记录回答，Agent 自己写的名片说了不算。

use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    private_rooms::PrivateRoomCapability,
    rooms::{RoomCatalogKind, RoomCatalogStatus},
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, PortFuture, PrivateRoomStore, RoomAgentOwnerRecord, RoomDirectory,
        RoomPeopleDirectory, RoomPersonName,
    },
};

/// 一次最多问这么多个 Agent；网页按这个数分批，和网络 Agent 标记一样。
pub const MAX_AGENT_OWNER_CANDIDATES: usize = 100;

/// 名字里带 `@` 的不整个给出去：早期账户没填昵称时用的是登录名，而登录名是邮箱。
const FALLBACK_DISPLAY_NAME: &str = "Agent Room User";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRoomPeople {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupRoomAgentOwners {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub agent_ids: Vec<AgentId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPeopleFailure {
    /// 登录过期，或者不是这个私人房间已加入的成员。
    Forbidden,
    /// 没有这个房间，或者它不是公共大厅、私人房间。
    NotFound,
    /// 一次问的 Agent 太多。
    TooMany,
    DependencyUnavailable,
    Internal,
}

pub type RoomPeopleResult<T> = Result<T, RoomPeopleFailure>;

pub trait RoomPeopleUseCases: Send + Sync {
    fn people(
        &self,
        request: ListRoomPeople,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomPersonName>>>;

    fn agent_owners(
        &self,
        request: LookupRoomAgentOwners,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomAgentOwnerRecord>>>;
}

pub struct RoomPeopleDependencies {
    pub people: Arc<dyn RoomPeopleDirectory>,
    pub private_rooms: Arc<dyn PrivateRoomStore>,
    pub directory: Arc<dyn RoomDirectory>,
    pub clock: Arc<dyn Clock>,
}

pub struct RoomPeopleService {
    people: Arc<dyn RoomPeopleDirectory>,
    private_rooms: Arc<dyn PrivateRoomStore>,
    directory: Arc<dyn RoomDirectory>,
    clock: Arc<dyn Clock>,
}

impl RoomPeopleService {
    pub fn new(dependencies: RoomPeopleDependencies) -> Self {
        Self {
            people: dependencies.people,
            private_rooms: dependencies.private_rooms,
            directory: dependencies.directory,
            clock: dependencies.clock,
        }
    }

    async fn people_internal(
        &self,
        request: ListRoomPeople,
    ) -> RoomPeopleResult<Vec<RoomPersonName>> {
        self.ensure_active(&request.actor)?;
        self.ensure_private_member(&request.actor, request.catalog_id)
            .await?;
        let names = self
            .people
            .member_names(request.catalog_id)
            .await
            .map_err(|error| repository(&error))?;
        Ok(names.into_iter().map(shown).collect())
    }

    async fn agent_owners_internal(
        &self,
        request: LookupRoomAgentOwners,
    ) -> RoomPeopleResult<Vec<RoomAgentOwnerRecord>> {
        self.ensure_active(&request.actor)?;
        if request.agent_ids.len() > MAX_AGENT_OWNER_CANDIDATES {
            return Err(RoomPeopleFailure::TooMany);
        }
        let catalog = self
            .directory
            .find_catalog(request.catalog_id)
            .await
            .map_err(|error| repository(&error))?
            .ok_or(RoomPeopleFailure::NotFound)?;
        match catalog.kind() {
            // 公共大厅谁都能进，登录了就能看；停用了的大厅当作没有。
            RoomCatalogKind::PublicLobby if catalog.status() == RoomCatalogStatus::Active => {}
            RoomCatalogKind::PrivateRoom => {
                self.ensure_private_member(&request.actor, request.catalog_id)
                    .await?;
            }
            _ => return Err(RoomPeopleFailure::NotFound),
        }
        if request.agent_ids.is_empty() {
            return Ok(Vec::new());
        }
        let owners = self
            .people
            .agent_owners(request.catalog_id, &request.agent_ids)
            .await
            .map_err(|error| repository(&error))?;
        Ok(owners
            .into_iter()
            .map(|record| RoomAgentOwnerRecord {
                owner: record.owner.map(shown),
                admitted_by: record.admitted_by.map(shown),
                ..record
            })
            .collect())
    }

    fn ensure_active(&self, actor: &AuthenticatedPrincipal) -> RoomPeopleResult<()> {
        if actor.expires_at > self.clock.now() {
            Ok(())
        } else {
            Err(RoomPeopleFailure::Forbidden)
        }
    }

    /// 私人房间只给已加入、能查看的成员看。归档了的房间照样能看（归档保留查看）。不是成员的和没有这个
    /// 房间一样回答，不让人拿房间 ID 试探。
    async fn ensure_private_member(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
    ) -> RoomPeopleResult<()> {
        let snapshot = self
            .private_rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(&error))?
            .ok_or(RoomPeopleFailure::NotFound)?;
        if snapshot
            .room()
            .allows(actor.principal_id, PrivateRoomCapability::View)
        {
            Ok(())
        } else {
            Err(RoomPeopleFailure::NotFound)
        }
    }
}

/// 给别人看的名字：带 `@` 的只留 `@` 前面那段，空了就用默认名。
pub fn shown_display_name(display_name: &str) -> String {
    let visible = display_name
        .split_once('@')
        .map_or(display_name, |(local, _)| local)
        .trim();
    if visible.is_empty() {
        FALLBACK_DISPLAY_NAME.to_owned()
    } else {
        visible.to_owned()
    }
}

fn shown(mut person: RoomPersonName) -> RoomPersonName {
    person.display_name = shown_display_name(&person.display_name);
    person
}

const fn repository(error: &RepositoryError) -> RoomPeopleFailure {
    match error.kind() {
        RepositoryErrorKind::Unavailable => RoomPeopleFailure::DependencyUnavailable,
        RepositoryErrorKind::NotFound => RoomPeopleFailure::NotFound,
        RepositoryErrorKind::Forbidden => RoomPeopleFailure::Forbidden,
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => RoomPeopleFailure::Internal,
    }
}

impl RoomPeopleUseCases for RoomPeopleService {
    fn people(
        &self,
        request: ListRoomPeople,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomPersonName>>> {
        Box::pin(self.people_internal(request))
    }

    fn agent_owners(
        &self,
        request: LookupRoomAgentOwners,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomAgentOwnerRecord>>> {
        Box::pin(self.agent_owners_internal(request))
    }
}

#[cfg(test)]
mod tests {
    use super::shown_display_name;

    #[test]
    fn 带_at_的名字只留前面那段() {
        assert_eq!(shown_display_name("小明"), "小明");
        assert_eq!(shown_display_name("ming@example.com"), "ming");
        assert_eq!(shown_display_name(" @example.com"), "Agent Room User");
        assert_eq!(shown_display_name("  Alice  "), "Alice");
    }
}
