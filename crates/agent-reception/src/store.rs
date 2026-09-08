use crate::{
    DeliveryStage, ReceiverBinding, ReceiverState, ReceptionFailure as Failure,
    ReceptionResult as Result, Resolution, model::validate_task_id,
};
use agent_room_agent_client::reception::ReceptionCheckpoint;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub struct ReceiverStore {
    _lock: File,
    path: PathBuf,
}

impl ReceiverStore {
    /// Hold an OS lock for the entire receiver lifetime, including host turns.
    /// # Errors
    /// Invalid task IDs, unavailable storage or another receiver owning this task.
    pub fn open(data_root: &Path, task_id: &str) -> Result<Self> {
        validate_task_id(task_id)?;
        let directory = data_root.join("receivers");
        fs::create_dir_all(&directory)
            .map_err(|_| Failure::local("receiver.storage_unavailable"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{task_id}.lock")))
            .map_err(|_| Failure::local("receiver.storage_unavailable"))?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => Failure::local("receiver.already_running"),
            fs::TryLockError::Error(_) => Failure::local("receiver.lock_unavailable"),
        })?;
        Ok(Self {
            _lock: lock,
            path: directory.join(format!("{task_id}.json")),
        })
    }

    /// # Errors
    /// Corrupt state is reported and never reset implicitly.
    pub fn load(&self) -> Result<Option<ReceiverState>> {
        read_state(&self.path)
    }

    /// Read an atomic snapshot without competing with the worker's lifetime lock.
    /// # Errors
    /// Invalid task IDs or corrupt/unavailable state.
    pub fn inspect(data_root: &Path, task_id: &str) -> Result<Option<ReceiverState>> {
        validate_task_id(task_id)?;
        read_state(&data_root.join("receivers").join(format!("{task_id}.json")))
    }

    /// # Errors
    /// An unreadable entry fails the listing visibly instead of hiding a receiver.
    pub fn list(data_root: &Path) -> Result<Vec<ReceiverState>> {
        let entries = match fs::read_dir(data_root.join("receivers")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(Failure::local("receiver.storage_unavailable")),
        };
        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| Failure::local("receiver.storage_unavailable"))?;
            if entry.path().extension().is_some_and(|ext| ext == "json") {
                if result.len() >= 128 {
                    return Err(Failure::local("receiver.limit_exceeded"));
                }
                if let Some(state) = read_state(&entry.path())? {
                    result.push(state);
                }
            }
        }
        result.sort_by(|a, b| a.binding.host.task_id.cmp(&b.binding.host.task_id));
        Ok(result)
    }

    /// Save binding and cursor together; renewing authorization never resets delivery.
    /// # Errors
    /// Moving a binding to another identity/room or changing its initial cursor is forbidden.
    pub fn configure(&self, binding: ReceiverBinding, service: &str) -> Result<ReceiverState> {
        binding.validate()?;
        let mut state = match self.load()? {
            Some(state) => {
                if !state.binding.same_identity(&binding) || state.bridge_service != service {
                    return Err(Failure::validation("receiver.identity_change_forbidden"));
                }
                state
            }
            None => ReceiverState {
                binding: binding.clone(),
                bridge_service: service.into(),
                agent_id: None,
                room_catalog_id: None,
                instance_id: None,
                checkpoint: ReceptionCheckpoint::Ready {
                    after_event_id: None,
                },
                enabled: false,
                last_delivery: None,
            },
        };
        state.binding = binding;
        self.save(&state)?;
        Ok(state)
    }

    /// # Errors
    /// Serialization or atomic durable replacement can fail.
    pub fn save(&self, state: &ReceiverState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Failure::local("receiver.storage_path_invalid"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| Failure::local("receiver.storage_unavailable"))?;
        serde_json::to_writer(&mut temporary, state)
            .map_err(|_| Failure::local("receiver.checkpoint_write_failed"))?;
        temporary
            .flush()
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| Failure::local("receiver.checkpoint_write_failed"))?;
        temporary
            .persist(&self.path)
            .map_err(|_| Failure::local("receiver.checkpoint_write_failed"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| Failure::local("receiver.checkpoint_write_failed"))?;
        Ok(())
    }

    /// # Errors
    /// Resolving a different or non-pending event is rejected.
    pub fn resolve(&self, event: &str, action: Resolution) -> Result<ReceiverState> {
        let mut state = self
            .load()?
            .ok_or_else(|| Failure::local("receiver.state_missing"))?;
        let ReceptionCheckpoint::Pending {
            after_event_id,
            event_id,
        } = &state.checkpoint
        else {
            return Err(Failure::validation("receiver.no_pending_delivery"));
        };
        if event != event_id {
            return Err(Failure::validation("receiver.event_mismatch"));
        }
        if matches!(action, Resolution::Retry) && state.last_delivery.is_none() {
            return Err(Failure::local("receiver.legacy_pending_review_required"));
        }
        state.checkpoint = ReceptionCheckpoint::Ready {
            after_event_id: match action {
                Resolution::Retry => after_event_id.clone(),
                Resolution::Skip => Some(event.to_owned()),
            },
        };
        if let Some(record) = &mut state.last_delivery {
            record.stage = match action {
                Resolution::Retry => DeliveryStage::Received,
                Resolution::Skip => DeliveryStage::Skipped,
            };
            record.failure = None;
        }
        self.save(&state)?;
        Ok(state)
    }

    /// # Errors
    /// Pending messages must be resolved explicitly before removing their cursor.
    pub fn remove(self) -> Result<()> {
        if self
            .load()?
            .is_some_and(|s| matches!(s.checkpoint, ReceptionCheckpoint::Pending { .. }))
        {
            return Err(Failure::local("receiver.pending_review_required"));
        }
        fs::remove_file(&self.path).map_err(|_| Failure::local("receiver.remove_failed"))
        // Keep the lock inode: deleting it could allow a second owner of a replaced lock file.
    }
}

fn read_state(path: &Path) -> Result<Option<ReceiverState>> {
    match File::open(path) {
        Ok(file) => {
            let state: ReceiverState = read_json(file)?;
            state.binding.validate()?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(&state.binding.host.task_id) {
                return Err(Failure::validation("receiver.state_identity_invalid"));
            }
            Ok(Some(state))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Failure::local("receiver.storage_unavailable")),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(file: File) -> Result<T> {
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::local("receiver.config_read_failed"))?;
    if bytes.len() > 65_536 {
        return Err(Failure::validation("receiver.config_too_large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::validation("receiver.config_invalid"))
}

/// # Errors
/// Malformed or oversized binding files are rejected before opening a session.
pub fn load_binding(path: &Path) -> Result<ReceiverBinding> {
    let binding: ReceiverBinding =
        read_json(File::open(path).map_err(|_| Failure::local("receiver.config_read_failed"))?)?;
    binding.validate()?;
    Ok(binding)
}
