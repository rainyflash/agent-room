mod crypto;
mod record;

use std::{fmt, path::Path};

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::handoffs::{
    ConsumedHandoffContext, HandoffReceiptRecord, HandoffRecordOutcome, HandoffStore,
    HandoffStoreCommand, HandoffStoreCommandOutcome, HandoffStoreFailure, HandoffStoreFailureKind,
    OneShotHandoffPackage,
};
use agent_room_domain::{
    handoff::{ContextHandoff, HandoffStatus},
    ids::{AgentInstanceId, HandoffId},
    time::UtcMillis,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::database::{SqliteBridgeStorageOpenFailure, open_handoff_pool};
use crypto::{EncryptedPackage, HandoffPackageCipher};

pub use crypto::{
    HANDOFF_STORAGE_KEY_BYTES, HandoffStorageKey, HandoffStorageKeyGenerationFailure,
};

#[derive(Clone)]
pub struct SqliteHandoffStore {
    pool: SqlitePool,
    cipher: HandoffPackageCipher,
}

impl SqliteHandoffStore {
    /// 打开独立的一次性 Handoff 数据库并迁移其专用结构。
    ///
    /// # Errors
    ///
    /// 目录不可创建、数据库不可连接或迁移失败时返回错误。
    pub async fn open(
        path: impl AsRef<Path>,
        key: HandoffStorageKey,
    ) -> Result<Self, SqliteBridgeStorageOpenFailure> {
        let pool = open_handoff_pool(path.as_ref()).await?;
        Ok(Self {
            pool,
            cipher: HandoffPackageCipher::new(&key),
        })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    async fn begin(&self) -> Result<Transaction<'_, Sqlite>, HandoffStoreFailure> {
        self.pool
            .begin()
            .await
            .map_err(|error| record::map_sqlx_error(&error))
    }

    async fn record(
        &self,
        handoff: &ContextHandoff,
        package: Option<&EncryptedPackage>,
    ) -> Result<HandoffRecordOutcome, HandoffStoreFailure> {
        let mut transaction = self.begin().await?;
        let inserted = record::insert_handoff(&mut transaction, handoff).await?;
        let outcome = if inserted {
            if let Some(package) = package {
                record::insert_package(&mut transaction, handoff.fields().id, package).await?;
            }
            HandoffRecordOutcome::Created(handoff.clone())
        } else {
            let existing = record::load_in_transaction(&mut transaction, handoff.fields().id)
                .await?
                .ok_or_else(corrupt_failure)?;
            if !record::same_intent(&existing.handoff, handoff) {
                return Err(conflict_failure());
            }
            if package.is_some() {
                ensure_existing_incoming_is_consistent(&mut transaction, &existing.handoff).await?;
            }
            HandoffRecordOutcome::Existing(existing.handoff)
        };
        commit(transaction).await?;
        Ok(outcome)
    }

    async fn apply_command(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
    ) -> Result<HandoffStoreCommandOutcome, HandoffStoreFailure> {
        let mut transaction = self.begin().await?;
        let loaded = record::load_in_transaction(&mut transaction, handoff_id)
            .await?
            .ok_or_else(not_found_failure)?;
        match command {
            HandoffStoreCommand::Consume {
                target_instance_id,
                occurred_at,
            } => {
                self.consume(transaction, loaded, target_instance_id, occurred_at)
                    .await
            }
            command => apply_non_consuming_command(transaction, loaded, command).await,
        }
    }

    async fn consume(
        &self,
        mut transaction: Transaction<'_, Sqlite>,
        mut loaded: record::LoadedHandoff,
        target_instance_id: AgentInstanceId,
        occurred_at: UtcMillis,
    ) -> Result<HandoffStoreCommandOutcome, HandoffStoreFailure> {
        ensure_target(&loaded.handoff, target_instance_id)?;
        match loaded.handoff.status() {
            HandoffStatus::Expired => return Err(expired_failure()),
            HandoffStatus::Delivered => {}
            _ => return Err(already_resolved_failure()),
        }
        if occurred_at >= loaded.handoff.fields().expires_at {
            loaded
                .handoff
                .expire(occurred_at)
                .map_err(|_| already_resolved_failure())?;
            record::update_handoff_state(&mut transaction, &loaded).await?;
            record::delete_package(&mut transaction, handoff_id(&loaded.handoff)).await?;
            commit(transaction).await?;
            return Err(expired_failure());
        }
        loaded
            .handoff
            .consume(occurred_at)
            .map_err(|_| already_resolved_failure())?;
        let stored_package = record::load_package(&mut transaction, handoff_id(&loaded.handoff))
            .await?
            .ok_or_else(corrupt_failure)?;
        let body = self.cipher.decrypt(
            &loaded.handoff,
            stored_package.key_version,
            &stored_package.nonce,
            &stored_package.ciphertext,
        )?;
        record::update_handoff_state(&mut transaction, &loaded).await?;
        record::delete_package(&mut transaction, handoff_id(&loaded.handoff)).await?;
        commit(transaction).await?;
        Ok(HandoffStoreCommandOutcome::Consumed(
            ConsumedHandoffContext::new(loaded.handoff, body),
        ))
    }

    async fn apply_receipt_record(
        &self,
        receipt: &HandoffReceiptRecord,
    ) -> Result<ContextHandoff, HandoffStoreFailure> {
        let mut transaction = self.begin().await?;
        let mut loaded = record::load_in_transaction(&mut transaction, receipt.handoff_id())
            .await?
            .ok_or_else(not_found_failure)?;
        let previous = loaded.handoff.clone();
        receipt
            .apply_to(&mut loaded.handoff)
            .map_err(|_| conflict_failure())?;
        if loaded.handoff != previous {
            record::update_handoff_state(&mut transaction, &loaded).await?;
        }
        if record::is_terminal(loaded.handoff.status()) {
            record::delete_package(&mut transaction, receipt.handoff_id()).await?;
        }
        commit(transaction).await?;
        Ok(loaded.handoff)
    }
}

impl fmt::Debug for SqliteHandoffStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteHandoffStore")
            .field("pool", &"SqlitePool")
            .field("cipher", &"[已隐藏]")
            .finish()
    }
}

impl HandoffStore for SqliteHandoffStore {
    fn find(
        &self,
        handoff_id: HandoffId,
    ) -> PortFuture<'_, Result<Option<ContextHandoff>, HandoffStoreFailure>> {
        Box::pin(async move {
            record::load_from_pool(&self.pool, handoff_id)
                .await
                .map(|loaded| loaded.map(|value| value.handoff))
        })
    }

    fn record_outgoing<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        Box::pin(async move { self.record(handoff, None).await })
    }

    fn accept_incoming<'a>(
        &'a self,
        handoff: &'a ContextHandoff,
        package: &'a OneShotHandoffPackage,
    ) -> PortFuture<'a, Result<HandoffRecordOutcome, HandoffStoreFailure>> {
        Box::pin(async move {
            if handoff.status() != HandoffStatus::Delivered {
                return Err(conflict_failure());
            }
            HandoffPackageCipher::validate_plaintext(handoff, package)?;
            let encrypted = self.cipher.encrypt(handoff, package.body().as_ref())?;
            self.record(handoff, Some(&encrypted)).await
        })
    }

    fn apply(
        &self,
        handoff_id: HandoffId,
        command: HandoffStoreCommand,
    ) -> PortFuture<'_, Result<HandoffStoreCommandOutcome, HandoffStoreFailure>> {
        Box::pin(async move { self.apply_command(handoff_id, command).await })
    }

    fn apply_receipt<'a>(
        &'a self,
        receipt: &'a HandoffReceiptRecord,
    ) -> PortFuture<'a, Result<ContextHandoff, HandoffStoreFailure>> {
        Box::pin(async move { self.apply_receipt_record(receipt).await })
    }
}

async fn ensure_existing_incoming_is_consistent(
    transaction: &mut Transaction<'_, Sqlite>,
    existing: &ContextHandoff,
) -> Result<(), HandoffStoreFailure> {
    let package_exists = record::load_package(transaction, existing.fields().id)
        .await?
        .is_some();
    match existing.status() {
        HandoffStatus::Delivered if package_exists => Ok(()),
        HandoffStatus::Delivered => Err(corrupt_failure()),
        status if record::is_terminal(status) && !package_exists => Ok(()),
        status if record::is_terminal(status) => Err(corrupt_failure()),
        HandoffStatus::Proposed | HandoffStatus::Approved => Err(conflict_failure()),
        HandoffStatus::Consumed
        | HandoffStatus::Declined
        | HandoffStatus::Revoked
        | HandoffStatus::Expired
        | HandoffStatus::Failed => unreachable!("终态已由守卫分支处理"),
    }
}

async fn apply_non_consuming_command(
    mut transaction: Transaction<'_, Sqlite>,
    mut loaded: record::LoadedHandoff,
    command: HandoffStoreCommand,
) -> Result<HandoffStoreCommandOutcome, HandoffStoreFailure> {
    let previous = loaded.handoff.clone();
    match command {
        HandoffStoreCommand::MarkDelivered { occurred_at } => loaded
            .handoff
            .mark_delivered(occurred_at)
            .map_err(|_| already_resolved_failure())?,
        HandoffStoreCommand::Decline {
            target_instance_id,
            occurred_at,
        } => {
            ensure_target(&loaded.handoff, target_instance_id)?;
            loaded
                .handoff
                .decline(occurred_at)
                .map_err(|_| already_resolved_failure())?;
        }
        HandoffStoreCommand::Revoke {
            target_instance_id,
            occurred_at,
        } => {
            ensure_target(&loaded.handoff, target_instance_id)?;
            loaded
                .handoff
                .revoke(occurred_at)
                .map_err(|_| already_resolved_failure())?;
        }
        HandoffStoreCommand::Expire {
            target_instance_id,
            occurred_at,
        } => {
            ensure_target(&loaded.handoff, target_instance_id)?;
            loaded
                .handoff
                .expire(occurred_at)
                .map_err(|_| already_resolved_failure())?;
        }
        HandoffStoreCommand::Fail { code, occurred_at } => {
            loaded
                .handoff
                .fail(code, occurred_at)
                .map_err(|_| already_resolved_failure())?;
        }
        HandoffStoreCommand::Consume { .. } => unreachable!("消费命令由独立事务路径处理"),
    }
    if loaded.handoff != previous {
        record::update_handoff_state(&mut transaction, &loaded).await?;
    }
    if record::is_terminal(loaded.handoff.status()) {
        record::delete_package(&mut transaction, handoff_id(&loaded.handoff)).await?;
    }
    commit(transaction).await?;
    Ok(HandoffStoreCommandOutcome::Updated(loaded.handoff))
}

fn ensure_target(
    handoff: &ContextHandoff,
    target_instance_id: AgentInstanceId,
) -> Result<(), HandoffStoreFailure> {
    if handoff.fields().target_instance_id == target_instance_id {
        Ok(())
    } else {
        Err(conflict_failure())
    }
}

const fn handoff_id(handoff: &ContextHandoff) -> HandoffId {
    handoff.fields().id
}

async fn commit(transaction: Transaction<'_, Sqlite>) -> Result<(), HandoffStoreFailure> {
    transaction
        .commit()
        .await
        .map_err(|error| record::map_sqlx_error(&error))
}

const fn conflict_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Conflict)
}

const fn not_found_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::NotFound)
}

const fn expired_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Expired)
}

const fn already_resolved_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::AlreadyResolved)
}

const fn corrupt_failure() -> HandoffStoreFailure {
    HandoffStoreFailure::new(HandoffStoreFailureKind::Corrupt)
}
