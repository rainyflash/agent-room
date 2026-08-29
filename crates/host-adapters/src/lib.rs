#![allow(clippy::missing_errors_doc)]

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
#[cfg(windows)]
use std::ffi::OsStr;
use std::{
    env, fmt, fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
    sync::Arc,
};
use tempfile::NamedTempFile;
use thiserror::Error;

const SERVER_NAME: &str = "agent_room";
const MAX_COMMAND_OUTPUT: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKind {
    Codex,
    ClaudeCode,
    Cursor,
}

impl fmt::Display for HostKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
        })
    }
}

impl FromStr for HostKind {
    type Err = HostFailure;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude-code" => Ok(Self::ClaudeCode),
            "cursor" => Ok(Self::Cursor),
            _ => Err(HostFailure::new("host.unsupported", false)),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigurationAction {
    Create,
    Replace,
    Unchanged,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDetection {
    pub host: HostKind,
    pub installed: bool,
    pub configurable: bool,
    pub mechanism: String,
    pub diagnostic_code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationPlan {
    pub host: HostKind,
    pub action: ConfigurationAction,
    pub target: String,
    pub original_digest: String,
    pub desired_digest: String,
    pub summary_code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReceipt {
    pub host: HostKind,
    pub changed: bool,
    pub resulting_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualHostConfiguration {
    pub server_name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct HostContext {
    pub home_dir: PathBuf,
    pub local_app_data: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub path_entries: Vec<PathBuf>,
    pub mcp_executable: PathBuf,
}

impl HostContext {
    pub fn from_environment(mcp_executable: PathBuf) -> Result<Self, HostFailure> {
        if !mcp_executable.is_absolute() || !mcp_executable.is_file() {
            return Err(HostFailure::new("mcp.executable_missing", false));
        }
        let home_dir = env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| HostFailure::new("host.home_missing", false))?;
        let path_entries = env::var_os("PATH")
            .map(|value| env::split_paths(&value).collect())
            .unwrap_or_default();
        Ok(Self {
            home_dir,
            local_app_data: env::var_os("LOCALAPPDATA").map(PathBuf::from),
            app_data: env::var_os("APPDATA").map(PathBuf::from),
            path_entries,
            mcp_executable,
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("宿主配置失败：{code}")]
pub struct HostFailure {
    code: String,
    retryable: bool,
}

impl HostFailure {
    fn new(code: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            retryable,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, executable: &Path, arguments: &[String]) -> Result<CommandOutput, HostFailure>;
}

#[derive(Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, executable: &Path, arguments: &[String]) -> Result<CommandOutput, HostFailure> {
        let mut command = platform_command(executable, arguments);
        let output = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| HostFailure::new("host.command_spawn_failed", true))?;
        if output.stdout.len() > MAX_COMMAND_OUTPUT {
            return Err(HostFailure::new("host.command_output_too_large", false));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| HostFailure::new("host.command_output_invalid", false))?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout,
        })
    }
}

#[cfg(windows)]
fn platform_command(executable: &Path, arguments: &[String]) -> Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let extension = executable.extension().and_then(OsStr::to_str);
    let mut command = if matches!(extension, Some("cmd" | "bat")) {
        let mut value = Command::new("cmd.exe");
        value.args(["/d", "/s", "/c"]);
        value.arg(executable);
        value
    } else {
        Command::new(executable)
    };
    command.args(arguments).creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn platform_command(executable: &Path, arguments: &[String]) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments);
    command
}

pub trait AgentHostAdapter: Send + Sync {
    fn kind(&self) -> HostKind;
    fn detect(&self, context: &HostContext) -> HostDetection;
    fn plan(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
    ) -> Result<ConfigurationPlan, HostFailure>;
    fn apply(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected_original_digest: &str,
    ) -> Result<ApplyReceipt, HostFailure>;
    fn remove(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected_original_digest: &str,
    ) -> Result<ApplyReceipt, HostFailure>;
}

pub struct HostConfigurator {
    context: HostContext,
    runner: Arc<dyn CommandRunner>,
    adapters: Vec<Box<dyn AgentHostAdapter>>,
}

impl HostConfigurator {
    pub fn system(context: HostContext) -> Self {
        Self::new(context, Arc::new(SystemCommandRunner))
    }

    pub fn new(context: HostContext, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            context,
            runner,
            adapters: vec![
                Box::new(CodexAdapter),
                Box::new(ClaudeAdapter),
                Box::new(CursorAdapter),
            ],
        }
    }

    pub fn detect_all(&self) -> Vec<HostDetection> {
        self.adapters
            .iter()
            .map(|adapter| adapter.detect(&self.context))
            .collect()
    }

    pub fn manual_configuration(&self) -> ManualHostConfiguration {
        ManualHostConfiguration {
            server_name: SERVER_NAME.into(),
            transport: "stdio".into(),
            command: self.context.mcp_executable.to_string_lossy().into_owned(),
            args: Vec::new(),
        }
    }

    pub fn plan(&self, host: HostKind) -> Result<ConfigurationPlan, HostFailure> {
        self.adapter(host)?
            .plan(&self.context, self.runner.as_ref())
    }

    pub fn apply(
        &self,
        host: HostKind,
        expected_original_digest: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        self.adapter(host)?.apply(
            &self.context,
            self.runner.as_ref(),
            expected_original_digest,
        )
    }

    pub fn remove(
        &self,
        host: HostKind,
        expected_original_digest: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        self.adapter(host)?.remove(
            &self.context,
            self.runner.as_ref(),
            expected_original_digest,
        )
    }

    fn adapter(&self, host: HostKind) -> Result<&dyn AgentHostAdapter, HostFailure> {
        self.adapters
            .iter()
            .find(|adapter| adapter.kind() == host)
            .map(AsRef::as_ref)
            .ok_or_else(|| HostFailure::new("host.unsupported", false))
    }
}

struct CodexAdapter;

impl AgentHostAdapter for CodexAdapter {
    fn kind(&self) -> HostKind {
        HostKind::Codex
    }

    fn detect(&self, context: &HostContext) -> HostDetection {
        detection(self.kind(), resolve_codex(context), "official-cli")
    }

    fn plan(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
    ) -> Result<ConfigurationPlan, HostFailure> {
        let executable =
            resolve_codex(context).ok_or_else(|| HostFailure::new("codex.not_installed", false))?;
        let current = codex_state(runner, &executable)?;
        let desired = json!({"name": SERVER_NAME, "transport": {"type": "stdio", "command": context.mcp_executable, "args": []}});
        let unchanged = current.as_ref().is_some_and(|value| {
            value.pointer("/transport/type").and_then(Value::as_str) == Some("stdio")
                && value.pointer("/transport/command").and_then(Value::as_str)
                    == context.mcp_executable.to_str()
                && value
                    .pointer("/transport/args")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
        });
        Ok(plan_for(
            self.kind(),
            "Codex user MCP registry",
            current.as_ref(),
            &desired,
            unchanged,
        ))
    }

    fn apply(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Unchanged {
            return Ok(receipt(self.kind(), false, plan.desired_digest));
        }
        let executable =
            resolve_codex(context).ok_or_else(|| HostFailure::new("codex.not_installed", false))?;
        if plan.action == ConfigurationAction::Replace {
            require_success(
                runner.run(
                    &executable,
                    &["mcp".into(), "remove".into(), SERVER_NAME.into()],
                )?,
                "codex.remove_failed",
            )?;
        }
        let path = context.mcp_executable.to_string_lossy().into_owned();
        require_success(
            runner.run(
                &executable,
                &[
                    "mcp".into(),
                    "add".into(),
                    SERVER_NAME.into(),
                    "--".into(),
                    path,
                ],
            )?,
            "codex.add_failed",
        )?;
        let verified = self.plan(context, runner)?;
        if verified.action != ConfigurationAction::Unchanged {
            return Err(HostFailure::new("codex.verify_failed", true));
        }
        Ok(receipt(self.kind(), true, verified.desired_digest))
    }

    fn remove(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Create {
            return Ok(receipt(self.kind(), false, missing_digest()));
        }
        let executable =
            resolve_codex(context).ok_or_else(|| HostFailure::new("codex.not_installed", false))?;
        require_success(
            runner.run(
                &executable,
                &["mcp".into(), "remove".into(), SERVER_NAME.into()],
            )?,
            "codex.remove_failed",
        )?;
        Ok(receipt(self.kind(), true, missing_digest()))
    }
}

struct ClaudeAdapter;

impl AgentHostAdapter for ClaudeAdapter {
    fn kind(&self) -> HostKind {
        HostKind::ClaudeCode
    }

    fn detect(&self, context: &HostContext) -> HostDetection {
        detection(
            self.kind(),
            resolve_path_command(context, "claude"),
            "official-cli",
        )
    }

    fn plan(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
    ) -> Result<ConfigurationPlan, HostFailure> {
        let executable = resolve_path_command(context, "claude")
            .ok_or_else(|| HostFailure::new("claude.not_installed", false))?;
        let output = runner.run(
            &executable,
            &["mcp".into(), "get".into(), SERVER_NAME.into()],
        )?;
        let current = (output.status == 0).then_some(output.stdout);
        let desired =
            json!({"type": "stdio", "command": context.mcp_executable, "args": [], "env": {}});
        let path = context.mcp_executable.to_string_lossy();
        let unchanged = current
            .as_ref()
            .is_some_and(|value| value.contains(path.as_ref()));
        Ok(ConfigurationPlan {
            host: self.kind(),
            action: if unchanged {
                ConfigurationAction::Unchanged
            } else if current.is_some() {
                ConfigurationAction::Replace
            } else {
                ConfigurationAction::Create
            },
            target: "Claude Code user MCP registry".into(),
            original_digest: current.as_deref().map_or_else(missing_digest, digest_bytes),
            desired_digest: digest_value(&desired),
            summary_code: if unchanged {
                "host.already_configured"
            } else {
                "host.configuration_required"
            }
            .into(),
        })
    }

    fn apply(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Unchanged {
            return Ok(receipt(self.kind(), false, plan.desired_digest));
        }
        let executable = resolve_path_command(context, "claude")
            .ok_or_else(|| HostFailure::new("claude.not_installed", false))?;
        if plan.action == ConfigurationAction::Replace {
            require_success(
                runner.run(
                    &executable,
                    &[
                        "mcp".into(),
                        "remove".into(),
                        "--scope".into(),
                        "user".into(),
                        SERVER_NAME.into(),
                    ],
                )?,
                "claude.remove_failed",
            )?;
        }
        let config = serde_json::to_string(
            &json!({"type": "stdio", "command": context.mcp_executable, "args": [], "env": {}}),
        )
        .map_err(|_| HostFailure::new("host.serialization_failed", false))?;
        require_success(
            runner.run(
                &executable,
                &[
                    "mcp".into(),
                    "add-json".into(),
                    "--scope".into(),
                    "user".into(),
                    SERVER_NAME.into(),
                    config,
                ],
            )?,
            "claude.add_failed",
        )?;
        let verified = self.plan(context, runner)?;
        if verified.action != ConfigurationAction::Unchanged {
            return Err(HostFailure::new("claude.verify_failed", true));
        }
        Ok(receipt(self.kind(), true, verified.desired_digest))
    }

    fn remove(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Create {
            return Ok(receipt(self.kind(), false, missing_digest()));
        }
        let executable = resolve_path_command(context, "claude")
            .ok_or_else(|| HostFailure::new("claude.not_installed", false))?;
        require_success(
            runner.run(
                &executable,
                &[
                    "mcp".into(),
                    "remove".into(),
                    "--scope".into(),
                    "user".into(),
                    SERVER_NAME.into(),
                ],
            )?,
            "claude.remove_failed",
        )?;
        Ok(receipt(self.kind(), true, missing_digest()))
    }
}

struct CursorAdapter;

impl CursorAdapter {
    fn config_path(context: &HostContext) -> PathBuf {
        context.home_dir.join(".cursor").join("mcp.json")
    }

    fn state(context: &HostContext) -> Result<(Option<Vec<u8>>, Value), HostFailure> {
        let path = Self::config_path(context);
        let bytes = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(HostFailure::new("cursor.config_read_failed", true)),
        };
        let root = bytes.as_ref().map_or_else(
            || Ok(Value::Object(Map::new())),
            |value| {
                serde_json::from_slice(value)
                    .map_err(|_| HostFailure::new("cursor.config_invalid", false))
            },
        )?;
        if !root.is_object() {
            return Err(HostFailure::new("cursor.config_invalid", false));
        }
        Ok((bytes, root))
    }

    fn desired(context: &HostContext, mut root: Value, remove: bool) -> Result<Value, HostFailure> {
        let object = root
            .as_object_mut()
            .ok_or_else(|| HostFailure::new("cursor.config_invalid", false))?;
        let servers = object
            .entry("mcpServers")
            .or_insert_with(|| Value::Object(Map::new()));
        let servers = servers
            .as_object_mut()
            .ok_or_else(|| HostFailure::new("cursor.config_invalid", false))?;
        if remove {
            servers.remove(SERVER_NAME);
        } else {
            servers.insert(
                SERVER_NAME.into(),
                json!({"command": context.mcp_executable, "args": []}),
            );
        }
        Ok(root)
    }

    fn write(context: &HostContext, value: &Value) -> Result<(), HostFailure> {
        let path = Self::config_path(context);
        let parent = path
            .parent()
            .ok_or_else(|| HostFailure::new("cursor.config_path_invalid", false))?;
        fs::create_dir_all(parent)
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        if path.is_file() {
            fs::copy(&path, path.with_extension("json.agent-room.bak"))
                .map_err(|_| HostFailure::new("cursor.config_backup_failed", true))?;
        }
        let mut temporary = NamedTempFile::new_in(parent)
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        serde_json::to_writer_pretty(&mut temporary, value)
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        temporary
            .write_all(b"\n")
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        temporary
            .persist(&path)
            .map_err(|_| HostFailure::new("cursor.config_write_failed", true))?;
        Ok(())
    }
}

impl AgentHostAdapter for CursorAdapter {
    fn kind(&self) -> HostKind {
        HostKind::Cursor
    }

    fn detect(&self, context: &HostContext) -> HostDetection {
        let executable = resolve_path_command(context, "cursor").or_else(|| {
            context.local_app_data.as_ref().and_then(|root| {
                [
                    root.join("Programs/Cursor/Cursor.exe"),
                    root.join("Programs/cursor/Cursor.exe"),
                ]
                .into_iter()
                .find(|path| path.is_file())
            })
        });
        detection(self.kind(), executable, "documented-json")
    }

    fn plan(
        &self,
        context: &HostContext,
        _runner: &dyn CommandRunner,
    ) -> Result<ConfigurationPlan, HostFailure> {
        if !self.detect(context).installed {
            return Ok(ConfigurationPlan {
                host: self.kind(),
                action: ConfigurationAction::Unavailable,
                target: "~/.cursor/mcp.json".into(),
                original_digest: missing_digest(),
                desired_digest: missing_digest(),
                summary_code: "cursor.not_installed".into(),
            });
        }
        let (bytes, root) = Self::state(context)?;
        let desired = Self::desired(context, root.clone(), false)?;
        let unchanged = root == desired;
        Ok(ConfigurationPlan {
            host: self.kind(),
            action: if unchanged {
                ConfigurationAction::Unchanged
            } else if bytes.is_some() {
                ConfigurationAction::Replace
            } else {
                ConfigurationAction::Create
            },
            target: "~/.cursor/mcp.json".into(),
            original_digest: bytes.as_deref().map_or_else(missing_digest, digest_bytes),
            desired_digest: digest_value(&desired),
            summary_code: if unchanged {
                "host.already_configured"
            } else {
                "host.configuration_required"
            }
            .into(),
        })
    }

    fn apply(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Unavailable {
            return Err(HostFailure::new("cursor.not_installed", false));
        }
        if plan.action == ConfigurationAction::Unchanged {
            return Ok(receipt(self.kind(), false, plan.desired_digest));
        }
        let (_, root) = Self::state(context)?;
        let desired = Self::desired(context, root, false)?;
        Self::write(context, &desired)?;
        Ok(receipt(self.kind(), true, digest_value(&desired)))
    }

    fn remove(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        let plan = self.plan(context, runner)?;
        guard_digest(&plan, expected)?;
        let (_, root) = Self::state(context)?;
        let desired = Self::desired(context, root, true)?;
        Self::write(context, &desired)?;
        Ok(receipt(self.kind(), true, digest_value(&desired)))
    }
}

#[allow(clippy::needless_pass_by_value)]
fn detection(host: HostKind, executable: Option<PathBuf>, mechanism: &str) -> HostDetection {
    let installed = executable.is_some();
    HostDetection {
        host,
        installed,
        configurable: installed,
        mechanism: mechanism.into(),
        diagnostic_code: if installed {
            "host.detected"
        } else {
            "host.not_detected"
        }
        .into(),
    }
}

fn resolve_codex(context: &HostContext) -> Option<PathBuf> {
    env::var_os("CODEX_CLI_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| resolve_path_command(context, "codex"))
        .or_else(|| {
            let directory = context.local_app_data.as_ref()?.join("OpenAI/Codex/bin");
            let mut candidates = fs::read_dir(directory)
                .ok()?
                .flatten()
                .map(|entry| entry.path().join("codex.exe"))
                .filter(|path| path.is_file())
                .collect::<Vec<_>>();
            candidates.sort();
            candidates.pop()
        })
}

fn resolve_path_command(context: &HostContext, name: &str) -> Option<PathBuf> {
    let names = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
        ]
    } else {
        vec![name.to_owned()]
    };
    context
        .path_entries
        .iter()
        .flat_map(|directory| names.iter().map(move |file| directory.join(file)))
        .find(|path| path.is_file())
}

fn codex_state(
    runner: &dyn CommandRunner,
    executable: &Path,
) -> Result<Option<Value>, HostFailure> {
    let output = runner.run(executable, &["mcp".into(), "list".into(), "--json".into()])?;
    require_success(output.clone(), "codex.list_failed")?;
    let values: Vec<Value> = serde_json::from_str(&output.stdout)
        .map_err(|_| HostFailure::new("codex.list_invalid", false))?;
    Ok(values
        .into_iter()
        .find(|value| value.get("name").and_then(Value::as_str) == Some(SERVER_NAME)))
}

fn plan_for(
    host: HostKind,
    target: &str,
    current: Option<&Value>,
    desired: &Value,
    unchanged: bool,
) -> ConfigurationPlan {
    ConfigurationPlan {
        host,
        action: if unchanged {
            ConfigurationAction::Unchanged
        } else if current.is_some() {
            ConfigurationAction::Replace
        } else {
            ConfigurationAction::Create
        },
        target: target.into(),
        original_digest: current.map_or_else(missing_digest, digest_value),
        desired_digest: digest_value(desired),
        summary_code: if unchanged {
            "host.already_configured"
        } else {
            "host.configuration_required"
        }
        .into(),
    }
}

fn guard_digest(plan: &ConfigurationPlan, expected: &str) -> Result<(), HostFailure> {
    if plan.original_digest == expected {
        Ok(())
    } else {
        Err(HostFailure::new("host.concurrent_modification", true))
    }
}

#[allow(clippy::needless_pass_by_value)]
fn require_success(output: CommandOutput, code: &str) -> Result<(), HostFailure> {
    if output.status == 0 {
        Ok(())
    } else {
        Err(HostFailure::new(code, true))
    }
}

fn receipt(host: HostKind, changed: bool, resulting_digest: String) -> ApplyReceipt {
    ApplyReceipt {
        host,
        changed,
        resulting_digest,
    }
}

fn missing_digest() -> String {
    digest_bytes(b"agent-room:missing")
}

fn digest_value(value: &Value) -> String {
    serde_json::to_vec(value).map_or_else(
        |_| digest_bytes(b"agent-room:invalid"),
        |bytes| digest_bytes(&bytes),
    )
}

#[allow(clippy::format_collect)]
fn digest_bytes(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
    }

    impl FakeRunner {
        fn with(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            _executable: &Path,
            _arguments: &[String],
        ) -> Result<CommandOutput, HostFailure> {
            self.outputs
                .lock()
                .expect("测试锁不能中毒")
                .pop_front()
                .ok_or_else(|| HostFailure::new("test.output_missing", false))
        }
    }

    fn context(root: &Path) -> HostContext {
        let mcp = root.join("agent-room-mcp.exe");
        fs::write(&mcp, b"test").expect("测试 MCP 可写");
        HostContext {
            home_dir: root.into(),
            local_app_data: None,
            app_data: None,
            path_entries: vec![],
            mcp_executable: mcp,
        }
    }

    #[test]
    fn cursor_merge_preserves_unrelated_servers() {
        let directory = tempfile::tempdir().expect("临时目录可创建");
        let context = context(directory.path());
        let config = directory.path().join(".cursor/mcp.json");
        fs::create_dir_all(config.parent().expect("配置有父目录")).expect("父目录可创建");
        fs::write(
            &config,
            br#"{"mcpServers":{"other":{"command":"safe"}},"theme":"dark"}"#,
        )
        .expect("配置可写");
        let (_, root) = CursorAdapter::state(&context).expect("配置可读");
        let desired = CursorAdapter::desired(&context, root, false).expect("配置可合并");
        CursorAdapter::write(&context, &desired).expect("配置可原子写入");
        let written: Value =
            serde_json::from_slice(&fs::read(config).expect("配置可读")).expect("配置有效");
        assert_eq!(
            written
                .pointer("/mcpServers/other/command")
                .and_then(Value::as_str),
            Some("safe")
        );
        assert_eq!(written.get("theme").and_then(Value::as_str), Some("dark"));
    }

    #[test]
    fn codex_state_selects_only_agent_room_entry() {
        let output = CommandOutput { status: 0, stdout: r#"[{"name":"secret","env":{"KEY":"do-not-touch"}},{"name":"agent_room","transport":{"type":"stdio","command":"x","args":[]}}]"#.into() };
        let runner = FakeRunner::with(vec![output]);
        let state = codex_state(&runner, Path::new("codex.exe"))
            .expect("列表有效")
            .expect("目标存在");
        assert_eq!(state.get("name").and_then(Value::as_str), Some(SERVER_NAME));
        assert!(state.get("env").is_none());
    }

    #[test]
    fn digest_guard_rejects_concurrent_change() {
        let plan = ConfigurationPlan {
            host: HostKind::Cursor,
            action: ConfigurationAction::Create,
            target: String::new(),
            original_digest: "new".into(),
            desired_digest: String::new(),
            summary_code: String::new(),
        };
        assert_eq!(
            guard_digest(&plan, "old")
                .expect_err("摘要不同必须失败")
                .code(),
            "host.concurrent_modification"
        );
    }

    #[test]
    fn manual_configuration_exposes_only_the_bundled_stdio_boundary() {
        let directory = tempfile::tempdir().expect("临时目录可创建");
        let context = context(directory.path());
        let expected_command = context.mcp_executable.to_string_lossy().into_owned();
        let configurator = HostConfigurator::new(context, Arc::new(FakeRunner::default()));

        assert_eq!(
            configurator.manual_configuration(),
            ManualHostConfiguration {
                server_name: SERVER_NAME.into(),
                transport: "stdio".into(),
                command: expected_command,
                args: Vec::new(),
            }
        );
    }
}
