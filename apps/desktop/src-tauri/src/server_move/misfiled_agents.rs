//! 把以前被误收走的本机 Agent 放回原处。
//!
//! 全新安装的 Windows 桌面端（#123 到 Alpha 100）每次启动都把数据目录里排在 `logs` 前面的东西收进
//! `retired/<秒>/`，其中有 `host-agents`：每个本机 Agent 的签名钥匙、聊天登录、加密存储和消息库。
//! 服务器上那个 Agent 的连接还在、认的还是原来那把签名钥匙，所以同一个任务再来时，Bridge 新建的
//! 那一份（新的钥匙）登记不上（409 `agent_instance.register.instance`），那个 Agent 再也连不上。
//!
//! 被收走的那一份原样留在 `retired/` 里，它的钥匙由这台电脑的内容根密钥加密，根密钥没被删过。
//! 把它挪回 `host-agents/<Agent>`，Agent 就照原来的身份接着用，加密房间里以前的消息也还读得到。
//!
//! - 只认那种装法留下的归档：里面只有排在 `logs` 前面的东西。真换服务器时整套收走的归档里有
//!   `messages`、`runtime` 这些，属于上一台服务器，不碰。
//! - 同一个 Agent 在好几次归档里都有时，最早的那一份才是登记成了的，之后每次收走的都是被拒绝的重试。
//! - 眼前 `host-agents` 里要是也有它，同样是被拒绝的重试，挪进 `retired/<秒>-refused/` 留着。
//!   上次启动以后新建的 Agent 不在任何归档里，不动。
//! - 只挪目录，不删东西；挪不动的留在原处，下次不再试（记录已经写下）。

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use super::{LOG_DIRECTORY, RETIRED_DIRECTORY, create_private_directory, unix_seconds};

const HOST_AGENTS: &str = "host-agents";
/// 每个本机 Agent 自己的加密文件夹，见 Bridge 的 `AgentSecretVault`。
const AGENT_SECRETS: &str = "secrets";
/// 被拒绝的重试挪到这里；不是纯数字的名字，不会被当成归档。
const REFUSED_SUFFIX: &str = "-refused";

/// 放回了几个、把几份被拒绝的重试挪开了、有几个没挪成。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Restored {
    pub(crate) put_back: usize,
    pub(crate) set_aside: usize,
    pub(crate) failed: usize,
}

/// 这台电脑上以前有没有走过归档那一步：`retired/` 下有以秒数命名的目录。
pub(crate) fn archived_before(data_root: &Path) -> bool {
    !archives(data_root).is_empty()
}

/// 调用方已经确认没有 Bridge 在运行。
pub(crate) fn restore(data_root: &Path) -> Restored {
    let mut summary = Restored::default();
    let registered = registered_copies(data_root);
    if registered.is_empty() {
        return summary;
    }
    let live_root = data_root.join(HOST_AGENTS);
    let refused_root = data_root
        .join(RETIRED_DIRECTORY)
        .join(format!("{}{REFUSED_SUFFIX}", unix_seconds()))
        .join(HOST_AGENTS);
    if create_private_directory(&live_root).is_err() {
        summary.failed = registered.len();
        return summary;
    }
    for (agent, source) in registered {
        let live = live_root.join(&agent);
        let refused = refused_root.join(&agent);
        let moved_aside = fs::symlink_metadata(&live).is_ok();
        if moved_aside
            && (create_private_directory(&refused_root).is_err()
                || fs::rename(&live, &refused).is_err())
        {
            summary.failed += 1;
            continue;
        }
        if fs::rename(&source, &live).is_ok() {
            summary.put_back += 1;
            summary.set_aside += usize::from(moved_aside);
        } else {
            summary.failed += 1;
            if moved_aside {
                // 放不回来就把眼前那一份挪回去，别让这个 Agent 两头都没有。
                let _ = fs::rename(&refused, &live);
            }
        }
    }
    summary
}

/// 每个被误收走的 Agent 登记成了的那一份：最早那次归档里的。
fn registered_copies(data_root: &Path) -> BTreeMap<OsString, PathBuf> {
    let mut registered = BTreeMap::new();
    for archive in archives(data_root) {
        if !only_partly_moved(&archive) {
            continue;
        }
        let Ok(agents) = fs::read_dir(archive.join(HOST_AGENTS)) else {
            continue;
        };
        for agent in agents.flatten() {
            let name = agent.file_name();
            if is_agent_id(&name) && holds_keys(&agent.path()) {
                registered.entry(name).or_insert_with(|| agent.path());
            }
        }
    }
    registered
}

/// `retired/` 下以秒数命名的目录，从早到晚。
fn archives(data_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(data_root.join(RETIRED_DIRECTORY)) else {
        return Vec::new();
    };
    let mut found: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let all_digits = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit());
            let seconds = all_digits.then(|| name.parse::<u64>().ok()).flatten()?;
            Some((seconds, entry.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// 那种装法每次都在挪到 `logs` 时停下，归档里只有排在它前面的名字。
fn only_partly_moved(archive: &Path) -> bool {
    fs::read_dir(archive).is_ok_and(|entries| {
        entries
            .flatten()
            .all(|entry| entry.file_name().as_os_str() < std::ffi::OsStr::new(LOG_DIRECTORY))
    })
}

fn is_agent_id(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
}

/// 没有钥匙的只是个空壳，放回去也登记不上。
fn holds_keys(agent: &Path) -> bool {
    fs::read_dir(agent.join(AGENT_SECRETS)).is_ok_and(|mut entries| entries.next().is_some())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{Restored, archived_before, restore};

    const FIRST: &str = "3f2b0c1e-58a4-4f0e-9a3b-6f0d2a7c1e10";
    const SECOND: &str = "7a1d9e42-0b6c-4c58-8f21-4d5e6f708192";

    fn agent(root: &Path, place: &str, id: &str, key: &str) {
        let folder = root.join(place).join("host-agents").join(id);
        fs::create_dir_all(folder.join("secrets")).expect("目录可创建");
        fs::write(folder.join("secrets").join("seed.json"), key).expect("钥匙可写");
    }

    fn key_of(root: &Path, place: &str, id: &str) -> Option<String> {
        fs::read_to_string(
            root.join(place)
                .join("host-agents")
                .join(id)
                .join("secrets")
                .join("seed.json"),
        )
        .ok()
    }

    #[test]
    fn 最早那次归档里的放回原处_之后被拒绝的重试都不用() {
        let root = tempdir().expect("临时目录有效");
        agent(root.path(), "retired/1700000100", FIRST, "登记成了的");
        agent(root.path(), "retired/1700000200", FIRST, "第一次重试");
        agent(root.path(), "", FIRST, "眼前的重试");

        let summary = restore(root.path());

        assert_eq!(
            summary,
            Restored {
                put_back: 1,
                set_aside: 1,
                failed: 0
            }
        );
        assert_eq!(
            key_of(root.path(), "", FIRST).as_deref(),
            Some("登记成了的")
        );
        assert!(key_of(root.path(), "retired/1700000100", FIRST).is_none());
        assert_eq!(
            key_of(root.path(), "retired/1700000200", FIRST).as_deref(),
            Some("第一次重试")
        );
        let refused: Vec<_> = fs::read_dir(root.path().join("retired"))
            .expect("归档目录可读")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with("-refused"))
            .collect();
        assert_eq!(refused.len(), 1);
        let kept = refused[0]
            .path()
            .join("host-agents")
            .join(FIRST)
            .join("secrets")
            .join("seed.json");
        assert_eq!(
            fs::read_to_string(kept).expect("挪开的那一份还在"),
            "眼前的重试"
        );
    }

    #[test]
    fn 没被收走过的本机_agent_不动_只在归档里的放回来() {
        let root = tempdir().expect("临时目录有效");
        agent(root.path(), "retired/1700000100", FIRST, "被收走的");
        agent(root.path(), "", SECOND, "上次启动以后新建的");

        let summary = restore(root.path());

        assert_eq!(
            summary,
            Restored {
                put_back: 1,
                set_aside: 0,
                failed: 0
            }
        );
        assert_eq!(key_of(root.path(), "", FIRST).as_deref(), Some("被收走的"));
        assert_eq!(
            key_of(root.path(), "", SECOND).as_deref(),
            Some("上次启动以后新建的")
        );
        assert!(
            !fs::read_dir(root.path().join("retired"))
                .expect("归档目录可读")
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with("-refused")),
            "没有要挪开的就不建目录"
        );
    }

    #[test]
    fn 真换服务器时整套收走的归档不碰() {
        let root = tempdir().expect("临时目录有效");
        agent(root.path(), "retired/1700000100", FIRST, "上一台服务器的");
        fs::create_dir_all(root.path().join("retired/1700000100/messages")).expect("目录可创建");

        assert_eq!(restore(root.path()), Restored::default());
        assert!(key_of(root.path(), "", FIRST).is_none());
        assert_eq!(
            key_of(root.path(), "retired/1700000100", FIRST).as_deref(),
            Some("上一台服务器的")
        );
    }

    #[test]
    fn 没有钥匙的空壳和不是_agent_的目录不放回() {
        let root = tempdir().expect("临时目录有效");
        let archive = root.path().join("retired/1700000100/host-agents");
        fs::create_dir_all(archive.join(FIRST).join("matrix-store")).expect("目录可创建");
        fs::create_dir_all(archive.join("not-an-agent").join("secrets")).expect("目录可创建");
        fs::write(archive.join("not-an-agent/secrets/seed.json"), "x").expect("可写");

        assert_eq!(restore(root.path()), Restored::default());
        assert!(!root.path().join("host-agents").exists());
    }

    #[test]
    fn 只有以秒数命名的目录算归档() {
        let root = tempdir().expect("临时目录有效");
        assert!(!archived_before(root.path()));
        fs::create_dir_all(root.path().join("retired/1700000000000-account")).expect("可创建");
        fs::create_dir_all(root.path().join("retired/1700000000-refused")).expect("可创建");
        fs::write(root.path().join("retired/1700000001"), "不是目录").expect("可写");
        assert!(!archived_before(root.path()));
        fs::create_dir_all(root.path().join("retired/1700000002")).expect("可创建");
        assert!(archived_before(root.path()));
    }
}
