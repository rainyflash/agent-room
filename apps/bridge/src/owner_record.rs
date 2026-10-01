//! 这台电脑上 Bridge 的主人（授权这台设备的账号）存成一个小文件：只有账号标识，不是凭据。
//! 重启以后不用等下一次刷新设备就知道主人是谁；写不进去只是少了“主人说话总能叫醒”。

use std::{
    path::PathBuf,
    sync::{Mutex, PoisonError},
};

use agent_room_bridge_core::ports::{BridgeOwner, BridgeOwnerRecord};
use serde::{Deserialize, Serialize};

pub(crate) struct FileOwnerRecord {
    path: PathBuf,
    owner: Mutex<Option<BridgeOwner>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredOwner {
    principal_id: String,
    matrix_user_id: String,
}

impl FileOwnerRecord {
    /// 读出之前记下的主人；没有或读不出来就当还不知道。
    pub(crate) fn open(path: PathBuf) -> Self {
        let owner = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoredOwner>(&bytes).ok())
            .map(|stored| BridgeOwner {
                principal_id: stored.principal_id,
                matrix_user_id: stored.matrix_user_id,
            });
        Self {
            path,
            owner: Mutex::new(owner),
        }
    }

    fn write(&self, owner: &BridgeOwner) -> std::io::Result<()> {
        let stored = StoredOwner {
            principal_id: owner.principal_id.clone(),
            matrix_user_id: owner.matrix_user_id.clone(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, &self.path)
    }
}

impl BridgeOwnerRecord for FileOwnerRecord {
    fn remember(&self, owner: &BridgeOwner) {
        let mut cached = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        if cached.as_ref() == Some(owner) {
            return;
        }
        if let Err(error) = self.write(owner) {
            tracing::warn!(%error, "没能记下这台电脑的主人；重启以后要等下一次刷新设备才认得");
        }
        *cached = Some(owner.clone());
    }

    fn owner(&self) -> Option<BridgeOwner> {
        self.owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ports::{BridgeOwner, BridgeOwnerRecord};

    use super::FileOwnerRecord;

    #[test]
    fn 记下的主人重启以后照样认得_读不出来就当不知道() {
        let directory = tempfile::tempdir().expect("临时目录可用");
        let path = directory.path().join("owner.json");
        let record = FileOwnerRecord::open(path.clone());
        assert_eq!(record.owner(), None);
        let owner = BridgeOwner {
            principal_id: "01945c1e-7b5a-7c7f-8a28-2de53f56a9a7".into(),
            matrix_user_id: "@owner:matrix.test".into(),
        };
        record.remember(&owner);
        assert_eq!(record.owner(), Some(owner.clone()));
        assert_eq!(FileOwnerRecord::open(path.clone()).owner(), Some(owner));

        std::fs::write(&path, b"not json").expect("可以写坏");
        assert_eq!(FileOwnerRecord::open(path).owner(), None);
    }
}
