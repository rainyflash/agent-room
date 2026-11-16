//! 项目房间流程测试用的假实现：内存里的库、照脚本回答的 GitHub、记下请求的建房，和拨得动的钟。
//! 库的规则照 `PostgreSQL` 那一份写（`crates/postgres-adapter/src/rooms/repository_claims.rs`）。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        BindRepository, BindRepositoryOutcome, BoundRepositoryRoom, Clock, CreateProjectRoom,
        DueClaimQuery, IdentifierFactory, PortFuture, ProjectRoomCandidate, ProjectRoomCreation,
        ProjectRoomCreator, ProjectRoomStore, RepositoryBindingRecord, RepositoryCheckRecord,
        RepositoryClaimCheckStart, RepositoryClaimInsertOutcome, RepositoryClaimRecord,
        RepositoryLinkReader, RepositoryReadFailure, RepositoryScan, SecretDigest, SecretFactory,
        SecretGenerationFailure, SecretValue, UnbindRepositoryOutcome,
    },
};
use agent_room_domain::{
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, PrincipalId, RepositoryClaimId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// 内存里的认领、房间和绑定。
#[derive(Default)]
pub struct MemoryStore {
    pub claims: Mutex<Vec<RepositoryClaimRecord>>,
    pub rooms: Mutex<HashMap<RoomCatalogId, ProjectRoomCandidate>>,
    pub bindings: Mutex<Vec<RepositoryBindingRecord>>,
}

impl MemoryStore {
    pub fn add_room(&self, room: ProjectRoomCandidate) {
        self.rooms.lock().unwrap().insert(room.catalog_id, room);
    }

    pub fn bind(&self, binding: RepositoryBindingRecord) {
        let catalog_id = binding.catalog_id;
        let repository = binding.repository.clone();
        self.bindings.lock().unwrap().push(binding);
        if let Some(room) = self.rooms.lock().unwrap().get_mut(&catalog_id) {
            room.bound = Some(repository);
        }
    }

    pub fn claim_of(&self, claim_id: RepositoryClaimId) -> Option<RepositoryClaimRecord> {
        self.claims
            .lock()
            .unwrap()
            .iter()
            .find(|claim| claim.id == claim_id)
            .cloned()
    }

    pub fn binding_of(&self, catalog_id: RoomCatalogId) -> Option<RepositoryBindingRecord> {
        self.bindings
            .lock()
            .unwrap()
            .iter()
            .find(|binding| binding.catalog_id == catalog_id)
            .cloned()
    }

    /// 认上以后的那一步：挪走原来的绑定、写下新的、删掉认领。
    fn write_binding(&self, request: &BindRepository) {
        self.bindings.lock().unwrap().retain(|binding| {
            Some(binding.catalog_id) != request.replacing
                || !binding.repository.same_as(&request.repository)
        });
        if let Some(previous) = request.replacing
            && let Some(room) = self.rooms.lock().unwrap().get_mut(&previous)
        {
            room.bound = None;
        }
        self.bind(RepositoryBindingRecord {
            catalog_id: request.catalog_id,
            repository: request.repository.clone(),
            verified_by: request.verified_by,
            verified_at: request.verified_at,
            verified_file: request.verified_file,
        });
        self.claims
            .lock()
            .unwrap()
            .retain(|claim| claim.id != request.claim_id);
    }

    /// 别的房间绑着这个仓库（挪走的那间不算）。
    fn taken_elsewhere(&self, request: &BindRepository) -> bool {
        self.bindings.lock().unwrap().iter().any(|binding| {
            binding.repository.same_as(&request.repository)
                && Some(binding.catalog_id) != request.replacing
        })
    }
}

impl ProjectRoomStore for MemoryStore {
    fn insert_claim<'a>(
        &'a self,
        claim: &'a RepositoryClaimRecord,
        max_pending: u32,
    ) -> PortFuture<'a, RepositoryResult<RepositoryClaimInsertOutcome>> {
        let mut claims = self.claims.lock().unwrap();
        let existing = claims.iter().position(|saved| {
            saved.principal_id == claim.principal_id && saved.repository.same_as(&claim.repository)
        });
        let outcome = if let Some(index) = existing
            && claims[index].target == claim.target
        {
            RepositoryClaimInsertOutcome::Existing(claims[index].clone())
        } else {
            if let Some(index) = existing {
                claims.remove(index);
            }
            if let Some(target) = claim.target {
                claims.retain(|saved| {
                    saved.principal_id != claim.principal_id || saved.target != Some(target)
                });
            }
            let mine = claims
                .iter()
                .filter(|saved| saved.principal_id == claim.principal_id)
                .count();
            if u32::try_from(mine).unwrap() >= max_pending {
                RepositoryClaimInsertOutcome::TooMany
            } else if claims
                .iter()
                .any(|saved| saved.share_code == claim.share_code)
            {
                RepositoryClaimInsertOutcome::ShareCodeTaken
            } else {
                claims.push(claim.clone());
                RepositoryClaimInsertOutcome::Inserted(claim.clone())
            }
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn claim(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryClaimRecord>>> {
        let claim = self.claim_of(claim_id);
        Box::pin(async move { Ok(claim) })
    }

    fn claims_of(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimRecord>>> {
        let claims = self
            .claims
            .lock()
            .unwrap()
            .iter()
            .filter(|claim| claim.principal_id == principal_id)
            .cloned()
            .collect();
        Box::pin(async move { Ok(claims) })
    }

    fn delete_claim(
        &self,
        claim_id: RepositoryClaimId,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let mut claims = self.claims.lock().unwrap();
        let before = claims.len();
        claims.retain(|claim| claim.id != claim_id || claim.principal_id != principal_id);
        let deleted = claims.len() < before;
        Box::pin(async move { Ok(deleted) })
    }

    fn start_check(
        &self,
        claim_id: RepositoryClaimId,
        now: UtcMillis,
        checked_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RepositoryClaimCheckStart>> {
        let mut claims = self.claims.lock().unwrap();
        let outcome = match claims.iter_mut().find(|claim| claim.id == claim_id) {
            None => RepositoryClaimCheckStart::Gone,
            Some(claim) => match claim.last_check {
                Some(last) if last.at > checked_before => RepositoryClaimCheckStart::TooSoon {
                    retry_at: UtcMillis::new(
                        last.at.value() + (now.value() - checked_before.value()),
                    )
                    .unwrap(),
                },
                last => {
                    claim.last_check = Some(RepositoryCheckRecord {
                        at: now,
                        result: last.map_or(ClaimCheckResult::Checking, |last| last.result),
                    });
                    claim.check_count += 1;
                    RepositoryClaimCheckStart::Started(claim.clone())
                }
            },
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn finish_check(
        &self,
        claim_id: RepositoryClaimId,
        started_at: UtcMillis,
        result: ClaimCheckResult,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        if let Some(claim) = self
            .claims
            .lock()
            .unwrap()
            .iter_mut()
            .find(|claim| claim.id == claim_id)
            && claim.last_check.map(|last| last.at) == Some(started_at)
        {
            claim.last_check = Some(RepositoryCheckRecord {
                at: started_at,
                result,
            });
        }
        Box::pin(async { Ok(()) })
    }

    fn due_claims(
        &self,
        query: DueClaimQuery,
    ) -> PortFuture<'_, RepositoryResult<Vec<RepositoryClaimId>>> {
        let mut due: Vec<_> = self
            .claims
            .lock()
            .unwrap()
            .iter()
            .map(|claim| {
                let last = claim.last_check.map_or(claim.claimed_at, |last| last.at);
                (claim, last)
            })
            .filter(|(claim, last)| {
                if claim.claimed_at > query.young_since {
                    *last <= query.young_checked_before
                } else {
                    *last <= query.old_checked_before
                }
            })
            .map(|(claim, last)| (last, claim.id))
            .collect();
        due.sort();
        let due = due
            .into_iter()
            .take(usize::try_from(query.limit).unwrap())
            .map(|(_, id)| id)
            .collect();
        Box::pin(async move { Ok(due) })
    }

    fn expire_claims(&self, claimed_before: UtcMillis) -> PortFuture<'_, RepositoryResult<u64>> {
        let mut claims = self.claims.lock().unwrap();
        let before = claims.len();
        claims.retain(|claim| claim.claimed_at > claimed_before);
        let expired = u64::try_from(before - claims.len()).unwrap();
        Box::pin(async move { Ok(expired) })
    }

    fn project_room_candidate(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<ProjectRoomCandidate>>> {
        let room = self.rooms.lock().unwrap().get(&catalog_id).cloned();
        Box::pin(async move { Ok(room) })
    }

    fn binding_for_room(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RepositoryBindingRecord>>> {
        let binding = self.binding_of(catalog_id);
        Box::pin(async move { Ok(binding) })
    }

    fn bound_room<'a>(
        &'a self,
        repository: &'a GithubRepository,
    ) -> PortFuture<'a, RepositoryResult<Option<BoundRepositoryRoom>>> {
        let binding = self
            .bindings
            .lock()
            .unwrap()
            .iter()
            .find(|binding| binding.repository.same_as(repository))
            .cloned();
        let bound = binding.map(|binding| {
            let room = self.rooms.lock().unwrap().get(&binding.catalog_id).cloned();
            BoundRepositoryRoom {
                share_code: room.as_ref().and_then(|room| room.share_code.clone()),
                held: room.is_some_and(|room| room.held),
                binding,
            }
        });
        Box::pin(async move { Ok(bound) })
    }

    fn bind_room<'a>(
        &'a self,
        request: &'a BindRepository,
    ) -> PortFuture<'a, RepositoryResult<BindRepositoryOutcome>> {
        let room = self.rooms.lock().unwrap().get(&request.catalog_id).cloned();
        let outcome = if self.claim_of(request.claim_id).is_none() {
            BindRepositoryOutcome::ClaimGone
        } else if !room.is_some_and(|room| {
            room.bindable_by(request.verified_by)
                && room.bound.is_none()
                && room.share_code.as_ref() == Some(&request.share_code)
        }) {
            BindRepositoryOutcome::RoomNotEligible
        } else if self.taken_elsewhere(request) {
            BindRepositoryOutcome::RepositoryTaken
        } else {
            self.write_binding(request);
            BindRepositoryOutcome::Bound
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn unbind_by_owner(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<UnbindRepositoryOutcome>> {
        let outcome = {
            let mut rooms = self.rooms.lock().unwrap();
            let other_unbound = rooms.values().any(|room| {
                room.catalog_id != catalog_id
                    && room.owner == Some(owner)
                    && room.listed
                    && room.active
                    && room.bound.is_none()
            });
            match rooms.get_mut(&catalog_id) {
                Some(room) if room.owner != Some(owner) => UnbindRepositoryOutcome::NotOwner,
                Some(room) if room.bound.is_none() => UnbindRepositoryOutcome::NotBound,
                Some(room) if room.listed && room.active && other_unbound => {
                    UnbindRepositoryOutcome::LimitReached
                }
                Some(room) => {
                    room.bound = None;
                    UnbindRepositoryOutcome::Unbound
                }
                None => UnbindRepositoryOutcome::NotOwner,
            }
        };
        if outcome == UnbindRepositoryOutcome::Unbound {
            self.bindings
                .lock()
                .unwrap()
                .retain(|binding| binding.catalog_id != catalog_id);
        }
        Box::pin(async move { Ok(outcome) })
    }

    fn refresh_binding<'a>(
        &'a self,
        binding: &'a RepositoryBindingRecord,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        let mut bindings = self.bindings.lock().unwrap();
        let saved = bindings.iter_mut().find(|saved| {
            saved.catalog_id == binding.catalog_id && saved.repository.same_as(&binding.repository)
        });
        let refreshed = saved.is_some();
        if let Some(saved) = saved {
            *saved = binding.clone();
        }
        Box::pin(async move { Ok(refreshed) })
    }
}

/// 照脚本回答的 GitHub：每个仓库里写着哪几个码；`failure` 设上时这次读不了；`paused_until` 设上时
/// 说现在不能读。
#[derive(Default)]
pub struct ScriptedReader {
    pub lines: Mutex<HashMap<String, Vec<(RoomShareCode, RepositoryFile)>>>,
    pub truncated: Mutex<bool>,
    pub failure: Mutex<Option<RepositoryReadFailure>>,
    pub paused_until: Mutex<Option<UtcMillis>>,
    pub scans: Mutex<Vec<(String, Vec<RoomShareCode>)>>,
}

impl ScriptedReader {
    pub fn write_line(&self, repository: &str, code: &RoomShareCode, file: RepositoryFile) {
        self.lines
            .lock()
            .unwrap()
            .entry(repository.to_ascii_lowercase())
            .or_default()
            .push((code.clone(), file));
    }

    pub fn remove_line(&self, repository: &str, code: &RoomShareCode) {
        if let Some(lines) = self
            .lines
            .lock()
            .unwrap()
            .get_mut(&repository.to_ascii_lowercase())
        {
            lines.retain(|(written, _)| written != code);
        }
    }

    pub fn scan_count(&self) -> usize {
        self.scans.lock().unwrap().len()
    }
}

impl RepositoryLinkReader for ScriptedReader {
    fn scan<'a>(
        &'a self,
        repository: &'a GithubRepository,
        codes: &'a [RoomShareCode],
    ) -> PortFuture<'a, Result<RepositoryScan, RepositoryReadFailure>> {
        let name = repository.full_name().to_ascii_lowercase();
        self.scans
            .lock()
            .unwrap()
            .push((name.clone(), codes.to_vec()));
        let failure = *self.failure.lock().unwrap();
        let result = if let Some(failure) = failure {
            Err(failure)
        } else {
            let lines = self.lines.lock().unwrap();
            let found = lines
                .get(&name)
                .into_iter()
                .flatten()
                .filter(|(code, _)| codes.contains(code))
                .cloned()
                .collect();
            Ok(RepositoryScan {
                found,
                truncated: *self.truncated.lock().unwrap(),
            })
        };
        Box::pin(async move { result })
    }

    fn next_read_at(&self) -> Option<UtcMillis> {
        *self.paused_until.lock().unwrap()
    }
}

/// 建项目房间：记下请求，在内存的库里照事务那样建房、绑上、删认领。`failure` 设上时回它。
pub struct RecordingCreator {
    pub store: Arc<MemoryStore>,
    pub requests: Mutex<Vec<CreateProjectRoom>>,
    pub failure: Mutex<Option<ProjectRoomCreation>>,
    pub next_room: RoomCatalogId,
}

impl ProjectRoomCreator for RecordingCreator {
    fn create_project_room(
        &self,
        request: CreateProjectRoom,
    ) -> PortFuture<'_, ProjectRoomCreation> {
        self.requests.lock().unwrap().push(request.clone());
        let outcome = if let Some(failure) = *self.failure.lock().unwrap() {
            failure
        } else if self.store.claim_of(request.claim_id).is_none() {
            ProjectRoomCreation::ClaimGone
        } else {
            self.store.add_room(ProjectRoomCandidate {
                catalog_id: self.next_room,
                owner: Some(request.owner),
                active: true,
                sub_room: false,
                listed: true,
                share_code: Some(request.share_code.clone()),
                held: false,
                bound: None,
            });
            self.store.write_binding(&BindRepository {
                claim_id: request.claim_id,
                catalog_id: self.next_room,
                repository: request.repository,
                verified_by: request.owner,
                verified_at: request.verified_at,
                verified_file: request.verified_file,
                share_code: request.share_code,
                replacing: request.replacing,
            });
            ProjectRoomCreation::Created(self.next_room)
        };
        Box::pin(async move { outcome })
    }
}

/// 拨得动的钟。
pub struct TestClock(pub Mutex<i64>);

impl TestClock {
    pub fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(*self.0.lock().unwrap()).unwrap()
    }
}

/// 每次给一个新的秘密，分享码就每次不一样。
#[derive(Default)]
pub struct CountingSecrets(Mutex<u64>);

impl SecretFactory for CountingSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("secret-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        SecretDigest::from_array(Sha256::digest(value.as_bytes()).into())
    }
}

pub struct TestIdentifiers;

impl IdentifierFactory for TestIdentifiers {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(Uuid::now_v7())
    }

    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(Uuid::now_v7())
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }

    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7())
    }

    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(Uuid::now_v7())
    }

    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(Uuid::now_v7())
    }

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
    }

    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(Uuid::now_v7())
    }

    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(Uuid::now_v7())
    }

    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_catalog_id(&self) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }

    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }

    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(Uuid::now_v7())
    }

    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(Uuid::now_v7())
    }
}
