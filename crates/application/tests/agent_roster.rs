use agent_room_application::{
    agent_roster::{AgentRosterPolicyPublisher, AgentRosterService},
    authentication::AuthenticatedPrincipal,
    moderation::ModerationFailureKind,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        ModerationAuthority, ModerationRoomContext, PortFuture,
    },
};
use agent_room_domain::{
    agent_lifecycle::AgentRosterPolicy,
    ids::{PrincipalId, RoomCatalogId},
    moderation::{ModerationRole, ModerationTarget},
    rooms::RoomCatalogKind,
    time::UtcMillis,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct Authority(ModerationRole, Vec<&'static str>);
impl Authority {
    fn one_room(role: ModerationRole) -> Self {
        Self(role, vec!["!managed:room.test"])
    }
}
impl ModerationAuthority for Authority {
    fn may_report<'a>(
        &'a self,
        _: PrincipalId,
        _: &'a ModerationTarget,
        _: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async { Ok(false) })
    }
    fn inspect_room<'a>(
        &'a self,
        _: PrincipalId,
        _: RoomCatalogId,
        _: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        Box::pin(async {
            Ok(Some(ModerationRoomContext {
                role: self.0,
                room_kind: RoomCatalogKind::PublicLobby,
                matrix_room_ids: self
                    .1
                    .iter()
                    .map(|room| MatrixRoomId::new(*room).unwrap())
                    .collect(),
                target_matrix_user_id: None,
            }))
        })
    }
    fn platform_role(&self, _: PrincipalId) -> PortFuture<'_, RepositoryResult<ModerationRole>> {
        Box::pin(async { Ok(self.0) })
    }
}
#[derive(Default)]
struct Publisher {
    writes: Mutex<Vec<(String, u16)>>,
    fail: bool,
}
impl AgentRosterPolicyPublisher for Publisher {
    fn publish<'a>(
        &'a self,
        room: &'a MatrixRoomId,
        policy: AgentRosterPolicy,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if self.fail {
                return Err(MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::DependencyUnavailable,
                ));
            }
            self.writes
                .lock()
                .unwrap()
                .push((room.as_str().to_owned(), policy.archive_after_days()));
            Ok(())
        })
    }
}
struct Now;
impl Clock for Now {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1000).unwrap()
    }
}
fn actor(expires: i64) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: "@human:room.test".to_owned(),
        display_name: "Human".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(0).unwrap(),
        expires_at: UtcMillis::new(expires).unwrap(),
        recently_authenticated: false,
    }
}

#[tokio::test]
async fn only_current_room_managers_may_change_the_shared_policy_without_reauthentication() {
    for (role, expires, allowed) in [
        (ModerationRole::RoomManager, 2000, true),
        (ModerationRole::None, 2000, false),
        (ModerationRole::AuditReader, 2000, false),
        (ModerationRole::RoomManager, 1000, false),
    ] {
        let publisher = Arc::new(Publisher::default());
        let service = AgentRosterService::new(
            Arc::new(Authority::one_room(role)),
            publisher.clone(),
            Arc::new(Now),
        );
        let result = service
            .update(
                actor(expires),
                RoomCatalogId::from_uuid(Uuid::now_v7()),
                AgentRosterPolicy::new(30).unwrap(),
            )
            .await;
        assert_eq!(result.is_ok(), allowed);
        assert_eq!(publisher.writes.lock().unwrap().len(), usize::from(allowed));
        if !allowed {
            assert_eq!(result.unwrap_err().kind(), ModerationFailureKind::Forbidden);
        }
    }
}

#[tokio::test]
async fn publication_failure_never_reports_the_rule_as_saved() {
    let publisher = Arc::new(Publisher {
        fail: true,
        ..Publisher::default()
    });
    let service = AgentRosterService::new(
        Arc::new(Authority::one_room(ModerationRole::RoomManager)),
        publisher.clone(),
        Arc::new(Now),
    );
    assert_eq!(
        service
            .update(
                actor(2000),
                RoomCatalogId::from_uuid(Uuid::now_v7()),
                AgentRosterPolicy::default()
            )
            .await
            .unwrap_err()
            .kind(),
        ModerationFailureKind::DependencyUnavailable
    );
    assert!(publisher.writes.lock().unwrap().is_empty());
    assert!(AgentRosterPolicy::new(0).is_none());
    assert!(AgentRosterPolicy::new(365).is_none());
}

#[tokio::test]
async fn public_lobby_rules_reach_every_active_shard() {
    let publisher = Arc::new(Publisher::default());
    let service = AgentRosterService::new(
        Arc::new(Authority(
            ModerationRole::PlatformModerator,
            vec!["!busy:room.test", "!quiet:room.test"],
        )),
        publisher.clone(),
        Arc::new(Now),
    );
    service
        .update(
            actor(2000),
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            AgentRosterPolicy::new(30).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        *publisher.writes.lock().unwrap(),
        [
            ("!busy:room.test".to_owned(), 30),
            ("!quiet:room.test".to_owned(), 30)
        ]
    );
}
