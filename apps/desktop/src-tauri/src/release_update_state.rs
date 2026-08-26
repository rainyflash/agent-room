use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use agent_room_release_manifest::{ReleaseChannel, ReleaseTrustState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub(crate) struct ReleaseUpdateStateStore {
    root: PathBuf,
}

impl ReleaseUpdateStateStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn trust_state(
        &self,
        channel: ReleaseChannel,
        installed_version: &str,
    ) -> Result<ReleaseTrustState, ReleaseUpdateStateFailure> {
        Ok(ReleaseTrustState {
            channel,
            highest_sequence: self.highest_committed_sequence(channel)?,
            installed_version: installed_version.to_owned(),
        })
    }

    pub(crate) fn record_pending(
        &self,
        channel: ReleaseChannel,
        sequence: u64,
        version: &str,
    ) -> Result<(), ReleaseUpdateStateFailure> {
        self.write_record(
            "pending",
            &UpdateStateRecord::new(channel, sequence, version),
        )
    }

    pub(crate) fn reconcile_installation(
        &self,
        installed_version: &str,
    ) -> Result<(), ReleaseUpdateStateFailure> {
        if !self.root.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.root).map_err(ReleaseUpdateStateFailure::Io)? {
            let entry = entry.map_err(ReleaseUpdateStateFailure::Io)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("pending-") || !name.ends_with(".json") {
                continue;
            }
            let record = read_record(&entry.path())?;
            validate_record_filename("pending", &name, &record)?;
            if record.version == installed_version {
                self.write_record("committed", &record)?;
            }
        }
        Ok(())
    }

    fn highest_committed_sequence(
        &self,
        channel: ReleaseChannel,
    ) -> Result<u64, ReleaseUpdateStateFailure> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut highest = 0;
        for entry in fs::read_dir(&self.root).map_err(ReleaseUpdateStateFailure::Io)? {
            let entry = entry.map_err(ReleaseUpdateStateFailure::Io)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("committed-") || !name.ends_with(".json") {
                continue;
            }
            let record = read_record(&entry.path())?;
            validate_record_filename("committed", &name, &record)?;
            if record.channel == channel {
                highest = highest.max(record.sequence);
            }
        }
        Ok(highest)
    }

    fn write_record(
        &self,
        phase: &str,
        record: &UpdateStateRecord,
    ) -> Result<(), ReleaseUpdateStateFailure> {
        fs::create_dir_all(&self.root).map_err(ReleaseUpdateStateFailure::Io)?;
        let path = self.root.join(record.filename(phase));
        let bytes = serde_json::to_vec(record).map_err(ReleaseUpdateStateFailure::Json)?;
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_record(&path)?;
                return if existing == *record {
                    Ok(())
                } else {
                    Err(ReleaseUpdateStateFailure::Corrupt)
                };
            }
            Err(error) => return Err(ReleaseUpdateStateFailure::Io(error)),
        };
        file.write_all(&bytes)
            .map_err(ReleaseUpdateStateFailure::Io)?;
        file.sync_all().map_err(ReleaseUpdateStateFailure::Io)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateStateRecord {
    schema_version: u32,
    channel: ReleaseChannel,
    sequence: u64,
    version: String,
}

impl UpdateStateRecord {
    fn new(channel: ReleaseChannel, sequence: u64, version: &str) -> Self {
        Self {
            schema_version: 1,
            channel,
            sequence,
            version: version.to_owned(),
        }
    }

    fn filename(&self, phase: &str) -> String {
        format!(
            "{phase}-{}-{}.json",
            channel_name(self.channel),
            self.sequence
        )
    }
}

fn read_record(path: &Path) -> Result<UpdateStateRecord, ReleaseUpdateStateFailure> {
    let bytes = fs::read(path).map_err(ReleaseUpdateStateFailure::Io)?;
    let record: UpdateStateRecord =
        serde_json::from_slice(&bytes).map_err(ReleaseUpdateStateFailure::Json)?;
    if record.schema_version != 1 || record.version.is_empty() {
        return Err(ReleaseUpdateStateFailure::Corrupt);
    }
    Ok(record)
}

fn validate_record_filename(
    phase: &str,
    filename: &str,
    record: &UpdateStateRecord,
) -> Result<(), ReleaseUpdateStateFailure> {
    if filename != record.filename(phase) {
        return Err(ReleaseUpdateStateFailure::Corrupt);
    }
    Ok(())
}

const fn channel_name(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "stable",
        ReleaseChannel::Testing => "testing",
    }
}

#[derive(Debug)]
pub(crate) enum ReleaseUpdateStateFailure {
    Corrupt,
    Io(io::Error),
    Json(serde_json::Error),
}

impl ReleaseUpdateStateFailure {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Corrupt => "desktop.update.state_corrupt",
            Self::Io(error) => {
                let _category = error.kind();
                "desktop.update.state_io_failed"
            }
            Self::Json(error) => {
                let _category = error.classify();
                "desktop.update.state_corrupt"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn 安装中断不会推进可信序号且成功启动后才提交() {
        let directory = tempdir().expect("必须能创建测试目录");
        let store = ReleaseUpdateStateStore::new(directory.path().to_path_buf());
        store
            .record_pending(ReleaseChannel::Stable, 12, "2.0.0")
            .expect("必须能记录待安装版本");

        assert_eq!(
            store
                .trust_state(ReleaseChannel::Stable, "1.0.0")
                .expect("必须能读取初始状态")
                .highest_sequence,
            0
        );
        store
            .reconcile_installation("1.0.0")
            .expect("中断安装应保持可重试");
        assert_eq!(
            store
                .trust_state(ReleaseChannel::Stable, "1.0.0")
                .expect("必须能读取中断状态")
                .highest_sequence,
            0
        );

        store
            .reconcile_installation("2.0.0")
            .expect("新版本启动后必须提交状态");
        assert_eq!(
            store
                .trust_state(ReleaseChannel::Stable, "2.0.0")
                .expect("必须能读取提交状态")
                .highest_sequence,
            12
        );
    }

    #[test]
    fn 渠道序号彼此隔离() {
        let directory = tempdir().expect("必须能创建测试目录");
        let store = ReleaseUpdateStateStore::new(directory.path().to_path_buf());
        store
            .record_pending(ReleaseChannel::Testing, 20, "3.0.0-beta.1")
            .expect("必须能记录测试渠道");
        store
            .reconcile_installation("3.0.0-beta.1")
            .expect("必须能提交测试渠道");

        assert_eq!(
            store
                .trust_state(ReleaseChannel::Stable, "2.0.0")
                .expect("必须能读取稳定渠道")
                .highest_sequence,
            0
        );
        assert_eq!(
            store
                .trust_state(ReleaseChannel::Testing, "3.0.0-beta.1")
                .expect("必须能读取测试渠道")
                .highest_sequence,
            20
        );
    }
}
