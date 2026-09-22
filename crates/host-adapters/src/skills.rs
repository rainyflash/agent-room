//! 把 Agent Room 的技能文件装进宿主的技能目录，让接入说明可以只剩一行命令。
//!
//! MCP 配置告诉宿主有哪些工具；技能告诉模型该怎么用它们。此前技能只随 Codex 插件走，
//! Claude Code 拿到的是 14 个裸工具，所以接入时只能把整套流程贴进任务里。

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{HostContext, HostFailure, HostKind, digest_bytes};

/// 技能目录名，也是技能名（front matter 的 `name`），两者必须一致。
pub const SKILL_NAME: &str = "agent-room";
const SKILL_FILENAME: &str = "SKILL.md";
const CLAUDE_CONFIG_DIR_VARIABLE: &str = "CLAUDE_CONFIG_DIR";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    /// 宿主没有技能目录这回事（Cursor），或桌面端没有带上技能文件。
    Unsupported,
    Missing,
    /// 已安装，但内容和本版桌面端带的不一样。
    Outdated,
    Current,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStatus {
    pub host: HostKind,
    pub state: SkillState,
    /// 技能文件的安装位置；不支持的宿主为空。
    pub target: Option<String>,
    pub bundled_digest: Option<String>,
    pub installed_digest: Option<String>,
}

/// 技能在宿主里的安装位置。
///
/// # Errors
///
/// 宿主不支持技能目录时返回 `host.skill_unsupported`。
pub fn skill_target(context: &HostContext, host: HostKind) -> Result<PathBuf, HostFailure> {
    match host {
        HostKind::ClaudeCode => Ok(claude_config_dir(context)
            .join("skills")
            .join(SKILL_NAME)
            .join(SKILL_FILENAME)),
        HostKind::Codex | HostKind::Cursor => {
            Err(HostFailure::new("host.skill_unsupported", false))
        }
    }
}

/// 读取本版桌面端带的技能文件与宿主里已安装的那份，比较摘要。
///
/// # Errors
///
/// 桌面端没有带技能文件，或已安装的文件读不出来时返回错误。
pub fn skill_status(context: &HostContext, host: HostKind) -> Result<SkillStatus, HostFailure> {
    let Ok(target) = skill_target(context, host) else {
        return Ok(unsupported(host));
    };
    let Some(bundled) = bundled_skill(context)? else {
        return Ok(unsupported(host));
    };
    let bundled_digest = digest_bytes(&bundled);
    let installed_digest = match fs::read(&target) {
        Ok(bytes) => Some(digest_bytes(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(HostFailure::new("host.skill_unreadable", true)),
    };
    let state = match installed_digest.as_deref() {
        None => SkillState::Missing,
        Some(installed) if installed == bundled_digest => SkillState::Current,
        Some(_) => SkillState::Outdated,
    };
    Ok(SkillStatus {
        host,
        state,
        target: Some(target.to_string_lossy().into_owned()),
        bundled_digest: Some(bundled_digest),
        installed_digest,
    })
}

/// 把本版的技能文件写进宿主的技能目录（先写临时文件再改名，读到一半的宿主不会看到半个文件）。
///
/// # Errors
///
/// 宿主不支持技能、桌面端没带技能文件，或目录/文件写不进去时返回错误。
pub fn install_skill(context: &HostContext, host: HostKind) -> Result<SkillStatus, HostFailure> {
    let target = skill_target(context, host)?;
    let bundled = bundled_skill(context)?
        .ok_or_else(|| HostFailure::new("host.skill_source_missing", false))?;
    let directory = target
        .parent()
        .ok_or_else(|| HostFailure::new("host.skill_target_invalid", false))?;
    fs::create_dir_all(directory).map_err(|_| HostFailure::new("host.skill_write_failed", true))?;
    let temporary = directory.join(format!("{SKILL_FILENAME}.agent-room-tmp"));
    fs::write(&temporary, &bundled)
        .map_err(|_| HostFailure::new("host.skill_write_failed", true))?;
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::remove_file(&temporary);
        // Windows 上目标已存在时 rename 会失败；直接覆盖再试一次。
        if fs::write(&target, &bundled).is_err() {
            let _ = error;
            return Err(HostFailure::new("host.skill_write_failed", true));
        }
    }
    let status = skill_status(context, host)?;
    if status.state != SkillState::Current {
        return Err(HostFailure::new("host.skill_verify_failed", true));
    }
    Ok(status)
}

fn unsupported(host: HostKind) -> SkillStatus {
    SkillStatus {
        host,
        state: SkillState::Unsupported,
        target: None,
        bundled_digest: None,
        installed_digest: None,
    }
}

fn bundled_skill(context: &HostContext) -> Result<Option<Vec<u8>>, HostFailure> {
    let Some(source) = context.skill_source.as_deref() else {
        return Ok(None);
    };
    match fs::read(source) {
        Ok(bytes) if !bytes.is_empty() => Ok(Some(bytes)),
        Ok(_) => Err(HostFailure::new("host.skill_source_invalid", false)),
        Err(_) => Err(HostFailure::new("host.skill_source_missing", false)),
    }
}

/// Claude Code 的配置目录：`CLAUDE_CONFIG_DIR` 优先，否则是 `~/.claude`。
fn claude_config_dir(context: &HostContext) -> PathBuf {
    context
        .claude_config_dir
        .clone()
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| context.home_dir.join(".claude"))
}

pub(crate) fn claude_config_dir_from_environment() -> Option<PathBuf> {
    std::env::var_os(CLAUDE_CONFIG_DIR_VARIABLE).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{SkillState, install_skill, skill_status, skill_target};
    use crate::{HostKind, tests::context};

    #[test]
    fn 技能装进_claude_目录并能识别缺失_过期_最新() {
        let directory = tempfile::tempdir().expect("临时目录可创建");
        let mut context = context(directory.path());
        let source = directory.path().join("bundle").join("SKILL.md");
        fs::create_dir_all(source.parent().expect("有父目录")).expect("可建目录");
        fs::write(&source, "---\nname: agent-room\n---\nv1\n").expect("技能可写");
        context.skill_source = Some(source.clone());

        let missing = skill_status(&context, HostKind::ClaudeCode).expect("状态可读");
        assert_eq!(missing.state, SkillState::Missing);
        assert_eq!(
            missing.target.as_deref().map(std::path::PathBuf::from),
            Some(directory.path().join(".claude/skills/agent-room/SKILL.md"))
        );

        let installed = install_skill(&context, HostKind::ClaudeCode).expect("可安装");
        assert_eq!(installed.state, SkillState::Current);
        assert_eq!(installed.installed_digest, installed.bundled_digest);
        assert_eq!(
            fs::read_to_string(skill_target(&context, HostKind::ClaudeCode).expect("有目标"))
                .expect("已安装文件可读"),
            "---\nname: agent-room\n---\nv1\n"
        );

        // 桌面端升级后技能变了：旧文件判为过期，再装一次就是最新。
        fs::write(&source, "---\nname: agent-room\n---\nv2\n").expect("技能可写");
        assert_eq!(
            skill_status(&context, HostKind::ClaudeCode)
                .expect("状态可读")
                .state,
            SkillState::Outdated
        );
        assert_eq!(
            install_skill(&context, HostKind::ClaudeCode)
                .expect("可覆盖安装")
                .state,
            SkillState::Current
        );
    }

    #[test]
    fn claude_config_dir_覆盖默认位置() {
        let directory = tempfile::tempdir().expect("临时目录可创建");
        let mut context = context(directory.path());
        context.claude_config_dir = Some(directory.path().join("elsewhere"));
        assert_eq!(
            skill_target(&context, HostKind::ClaudeCode).expect("有目标"),
            directory
                .path()
                .join("elsewhere/skills/agent-room/SKILL.md")
        );
    }

    #[test]
    fn 没有技能文件或不支持的宿主报为不支持_安装则报错() {
        let directory = tempfile::tempdir().expect("临时目录可创建");
        let context = context(directory.path());
        assert_eq!(
            skill_status(&context, HostKind::ClaudeCode)
                .expect("状态可读")
                .state,
            SkillState::Unsupported
        );
        assert_eq!(
            skill_status(&context, HostKind::Cursor)
                .expect("状态可读")
                .state,
            SkillState::Unsupported
        );
        assert_eq!(
            install_skill(&context, HostKind::ClaudeCode)
                .expect_err("没有技能文件不能安装")
                .code(),
            "host.skill_source_missing"
        );
        assert_eq!(
            install_skill(&context, HostKind::Cursor)
                .expect_err("Cursor 没有技能目录")
                .code(),
            "host.skill_unsupported"
        );
    }
}
