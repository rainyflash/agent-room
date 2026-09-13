use super::{HostContext, HostFailure, SERVER_NAME};
use std::{fs, io::Write as _};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, value};

pub(super) const TOOL_TIMEOUT_SECONDS: u32 = 86_400;

// `codex mcp add` has no persistent timeout option. Update only the documented field
// in the registered server, retaining comments, other servers and user settings.
pub(super) fn configure(context: &HostContext) -> Result<(), HostFailure> {
    let directory = context
        .codex_home
        .clone()
        .unwrap_or_else(|| context.home_dir.join(".codex"));
    if !directory.is_absolute() {
        return Err(HostFailure::new("codex.config_path_invalid", false));
    }
    let path = directory.join("config.toml");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| HostFailure::new("codex.config_read_failed", true))?;
    if !metadata.file_type().is_file() {
        return Err(HostFailure::new("codex.config_path_invalid", false));
    }
    let original = fs::read_to_string(&path)
        .map_err(|_| HostFailure::new("codex.config_read_failed", true))?;
    let mut document = original
        .parse::<DocumentMut>()
        .map_err(|_| HostFailure::new("codex.config_invalid", false))?;
    let server = document
        .get_mut("mcp_servers")
        .and_then(|servers| servers.get_mut(SERVER_NAME))
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| HostFailure::new("codex.verify_failed", true))?;
    if server.get("command").and_then(toml_edit::Item::as_str) != context.mcp_executable.to_str() {
        return Err(HostFailure::new("host.concurrent_modification", true));
    }
    if server.get("tool_timeout_sec").is_some_and(|item| {
        item.as_float()
            .is_some_and(|seconds| seconds >= f64::from(TOOL_TIMEOUT_SECONDS))
            || item
                .as_integer()
                .is_some_and(|seconds| seconds >= i64::from(TOOL_TIMEOUT_SECONDS))
    }) {
        return Ok(());
    }
    server.insert("tool_timeout_sec", value(i64::from(TOOL_TIMEOUT_SECONDS)));
    let mut temporary = NamedTempFile::new_in(&directory)
        .map_err(|_| HostFailure::new("codex.config_write_failed", true))?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|_| HostFailure::new("codex.config_write_failed", true))?;
    temporary
        .write_all(document.to_string().as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| HostFailure::new("codex.config_write_failed", true))?;
    if fs::read_to_string(&path).map_err(|_| HostFailure::new("codex.config_read_failed", true))?
        != original
    {
        return Err(HostFailure::new("host.concurrent_modification", true));
    }
    fs::copy(&path, path.with_extension("toml.agent-room.bak"))
        .map_err(|_| HostFailure::new("codex.config_backup_failed", true))?;
    temporary
        .persist(&path)
        .map_err(|_| HostFailure::new("codex.config_write_failed", true))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 更新指定配置目录的单个字段保留其他设置与更长期限() {
        let directory = tempfile::tempdir().unwrap();
        let mut context = crate::tests::context(directory.path());
        context.codex_home = Some(directory.path().join("custom-codex"));
        let target = context.codex_home.as_ref().unwrap();
        fs::create_dir_all(target).unwrap();
        let path = target.join("config.toml");
        let command = serde_json::to_string(context.mcp_executable.to_str().unwrap()).unwrap();
        let original = format!(
            "# user comment\nmodel = 'user-model'\n[mcp_servers.other]\ncommand = 'keep'\n[mcp_servers.agent_room]\ncommand = {command}\ntool_timeout_sec = 150\nenabled_tools = ['agent_room_wait_for_messages']\n"
        );
        fs::write(&path, &original).unwrap();
        configure(&context).unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        assert_eq!(
            updated,
            original.replace("tool_timeout_sec = 150", "tool_timeout_sec = 86400")
        );
        assert_eq!(
            fs::read_to_string(path.with_extension("toml.agent-room.bak")).unwrap(),
            original
        );
        let longer = updated.replace("86400", "172800");
        fs::write(&path, &longer).unwrap();
        configure(&context).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), longer);
        fs::write(&path, original.replace(&command, "'different-program'")).unwrap();
        assert_eq!(
            configure(&context).unwrap_err().code(),
            "host.concurrent_modification"
        );
    }
}
