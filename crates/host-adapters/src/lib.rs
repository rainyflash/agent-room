#![allow(clippy::missing_errors_doc)]

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    env, fmt, fs,
    io::Write as _,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tempfile::NamedTempFile;
use thiserror::Error;

const SERVER_NAME: &str = "agent_room";
mod codex_timeout;
mod command;
pub use command::SystemCommandRunner;

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
    pub codex_cli_path: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
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
            codex_cli_path: env::var_os("CODEX_CLI_PATH").map(PathBuf::from),
            codex_home: env::var_os("CODEX_HOME").map(PathBuf::from),
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
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, executable: &Path, arguments: &[String]) -> Result<CommandOutput, HostFailure>;
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

    /// Resolve a native executable for background reception without changing MCP configuration.
    /// # Errors
    /// The host is missing, incompatible, or only available through a shell wrapper.
    pub fn reception_executable(&self, host: HostKind) -> Result<PathBuf, HostFailure> {
        let executable = match host {
            HostKind::Codex => compatible_codex(&self.context, self.runner.as_ref())?.0,
            HostKind::ClaudeCode => path_commands(&self.context, "claude")
                .into_iter()
                .find(|path| {
                    !cfg!(windows)
                        || path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                })
                .ok_or_else(|| HostFailure::new("claude.not_installed", false))?,
            HostKind::Cursor => return Err(HostFailure::new("host.reception_unsupported", false)),
        };
        if cfg!(windows)
            && !executable
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Err(HostFailure::new("host.native_executable_required", false));
        }
        Ok(executable)
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
        detection(
            self.kind(),
            codex_candidates(context).into_iter().next(),
            "official-cli",
        )
    }

    fn plan(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
    ) -> Result<ConfigurationPlan, HostFailure> {
        let (_, current) = compatible_codex(context, runner)?;
        Ok(codex_plan(context, current.as_ref()))
    }

    fn apply(
        &self,
        context: &HostContext,
        runner: &dyn CommandRunner,
        expected: &str,
    ) -> Result<ApplyReceipt, HostFailure> {
        // Read, mutate and verify through the same compatible executable. A second
        // PATH lookup could select an older installation halfway through setup.
        let (executable, current) = compatible_codex(context, runner)?;
        let plan = codex_plan(context, current.as_ref());
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Unchanged {
            return Ok(receipt(self.kind(), false, plan.desired_digest));
        }
        let path = context.mcp_executable.to_string_lossy().into_owned();
        if !current
            .as_ref()
            .is_some_and(|current| codex_transport_matches(context, current))
        {
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
        }
        codex_timeout::configure(context)?;
        let verified = codex_plan(context, codex_state(runner, &executable)?.as_ref());
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
        let (executable, current) = compatible_codex(context, runner)?;
        let plan = codex_plan(context, current.as_ref());
        guard_digest(&plan, expected)?;
        if plan.action == ConfigurationAction::Create {
            return Ok(receipt(self.kind(), false, missing_digest()));
        }
        require_success(
            runner.run(
                &executable,
                &["mcp".into(), "remove".into(), SERVER_NAME.into()],
            )?,
            "codex.remove_failed",
        )?;
        if codex_state(runner, &executable)?.is_some() {
            return Err(HostFailure::new("codex.verify_failed", true));
        }
        Ok(receipt(self.kind(), true, missing_digest()))
    }
}

fn codex_plan(context: &HostContext, current: Option<&Value>) -> ConfigurationPlan {
    let desired = json!({"name": SERVER_NAME, "transport": {"type": "stdio", "command": context.mcp_executable, "args": []}, "tool_timeout_sec": codex_timeout::TOOL_TIMEOUT_SECONDS});
    let unchanged = current.is_some_and(|value| {
        codex_transport_matches(context, value)
            && value
                .get("tool_timeout_sec")
                .and_then(Value::as_f64)
                .is_some_and(|seconds| seconds >= f64::from(codex_timeout::TOOL_TIMEOUT_SECONDS))
    });
    plan_for(
        HostKind::Codex,
        "Codex user MCP registry",
        current,
        &desired,
        unchanged,
    )
}

fn codex_transport_matches(context: &HostContext, value: &Value) -> bool {
    value.get("enabled").and_then(Value::as_bool) != Some(false)
        && value.pointer("/transport/type").and_then(Value::as_str) == Some("stdio")
        && value.pointer("/transport/command").and_then(Value::as_str)
            == context.mcp_executable.to_str()
        && value
            .pointer("/transport/args")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
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

fn codex_candidates(context: &HostContext) -> Vec<PathBuf> {
    if let Some(path) = &context.codex_cli_path {
        // An explicit override must not silently configure another installation.
        return vec![path.clone()];
    }
    let mut bundled = context
        .local_app_data
        .as_ref()
        .and_then(|root| fs::read_dir(root.join("OpenAI/Codex/bin")).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("codex.exe"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    bundled.sort_by_key(|path| std::cmp::Reverse(path.metadata().and_then(|m| m.modified()).ok()));
    for path in path_commands(context, "codex") {
        if !bundled.contains(&path) {
            bundled.push(path);
        }
    }
    bundled
}

fn compatible_codex(
    context: &HostContext,
    runner: &dyn CommandRunner,
) -> Result<(PathBuf, Option<Value>), HostFailure> {
    let mut failure = None;
    for executable in codex_candidates(context) {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(HostFailure::new("codex.executable_invalid", false));
        }
        match codex_state(runner, &executable) {
            Ok(state) => return Ok((executable, state)),
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    Err(failure.unwrap_or_else(|| HostFailure::new("codex.not_installed", false)))
}

fn resolve_path_command(context: &HostContext, name: &str) -> Option<PathBuf> {
    path_commands(context, name).into_iter().next()
}

fn path_commands(context: &HostContext, name: &str) -> Vec<PathBuf> {
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
        .filter(|path| path.is_absolute() && path.is_file())
        .collect()
}

fn codex_state(
    runner: &dyn CommandRunner,
    executable: &Path,
) -> Result<Option<Value>, HostFailure> {
    let output = runner.run(executable, &["mcp".into(), "list".into(), "--json".into()])?;
    if output.status != 0 {
        let code = if output.stderr.contains("unknown variant")
            || output.stderr.contains("unknown field")
        {
            "codex.config_incompatible"
        } else if output.stderr.contains("failed to load configuration")
            || output.stderr.contains("Error loading config")
        {
            "codex.config_invalid"
        } else {
            "codex.list_failed"
        };
        return Err(HostFailure::new(code, true));
    }
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
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    }

    impl FakeRunner {
        fn with(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                calls: Mutex::default(),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            executable: &Path,
            arguments: &[String],
        ) -> Result<CommandOutput, HostFailure> {
            self.calls
                .lock()
                .unwrap()
                .push((executable.into(), arguments.to_vec()));
            self.outputs
                .lock()
                .expect("测试锁不能中毒")
                .pop_front()
                .ok_or_else(|| HostFailure::new("test.output_missing", false))
        }
    }

    pub(super) fn context(root: &Path) -> HostContext {
        let mcp = root.join("agent-room-mcp.exe");
        fs::write(&mcp, b"test").expect("测试 MCP 可写");
        HostContext {
            home_dir: root.into(),
            local_app_data: None,
            app_data: None,
            path_entries: vec![],
            mcp_executable: mcp,
            codex_cli_path: None,
            codex_home: None,
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
        let output = CommandOutput { status: 0, stderr: String::new(), stdout: r#"[{"name":"secret","env":{"KEY":"do-not-touch"}},{"name":"agent_room","transport":{"type":"stdio","command":"x","args":[]}}]"#.into() };
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

    fn output(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn installed_codex(root: &Path) -> (HostContext, PathBuf, PathBuf) {
        let mut context = context(root);
        let bundled = root.join("OpenAI/Codex/bin/new-build/codex.exe");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&bundled, b"native").unwrap();
        let npm = root.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
        fs::write(&npm, b"wrapper").unwrap();
        context.local_app_data = Some(root.into());
        context.path_entries.push(root.into());
        (context, bundled, npm)
    }

    #[test]
    fn desktop_codex_is_preferred_over_old_path_wrapper() {
        let directory = tempfile::tempdir().unwrap();
        let (context, bundled, _) = installed_codex(directory.path());
        let runner = FakeRunner::with(vec![output(0, "[]", "")]);
        let plan = CodexAdapter.plan(&context, &runner).unwrap();
        assert_eq!(plan.action, ConfigurationAction::Create);
        assert_eq!(runner.calls.lock().unwrap()[0].0, bundled);
    }

    #[test]
    fn background_reception_uses_bundled_codex_without_writing_mcp_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let (context, bundled, _) = installed_codex(directory.path());
        let runner = Arc::new(FakeRunner::with(vec![output(0, "[]", "")]));
        let configurator = HostConfigurator::new(context, runner.clone());
        assert_eq!(
            configurator.reception_executable(HostKind::Codex).unwrap(),
            bundled
        );
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, ["mcp", "list", "--json"]);
    }

    #[test]
    fn disabled_codex_entry_requires_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let (context, _, _) = installed_codex(directory.path());
        let entry = json!([{"name": SERVER_NAME, "enabled": false, "transport": {"type": "stdio", "command": context.mcp_executable, "args": []}}]);
        let runner = FakeRunner::with(vec![output(0, &entry.to_string(), "")]);
        assert_eq!(
            CodexAdapter.plan(&context, &runner).unwrap().action,
            ConfigurationAction::Replace
        );
    }

    #[test]
    fn incompatible_installation_falls_back_and_keeps_same_executable_for_write_and_verify() {
        let directory = tempfile::tempdir().unwrap();
        let (context, bundled, npm) = installed_codex(directory.path());
        let configured = json!([{"name": SERVER_NAME, "transport": {"type": "stdio", "command": context.mcp_executable, "args": []}, "tool_timeout_sec": 86400}]).to_string();
        let config = directory.path().join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            format!(
                "[mcp_servers.agent_room]\ncommand = {}\n",
                serde_json::to_string(context.mcp_executable.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();
        let runner = FakeRunner::with(vec![
            output(1, "", "unknown variant `max` secret-not-for-ui"),
            output(0, "[]", ""),
            output(0, "added", ""),
            output(0, &configured, ""),
        ]);
        assert!(
            CodexAdapter
                .apply(&context, &runner, &missing_digest())
                .unwrap()
                .changed
        );
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0, bundled);
        assert!(calls[1..].iter().all(|(path, _)| path == &npm));
        assert_eq!(calls[2].1[1], "add");
    }

    #[test]
    fn explicit_codex_override_is_respected_and_classified_without_disclosing_config() {
        let directory = tempfile::tempdir().unwrap();
        let (mut context, _, npm) = installed_codex(directory.path());
        context.codex_cli_path = Some(npm);
        let runner = FakeRunner::with(vec![output(
            1,
            "",
            "failed to load configuration: unknown variant `max`; secret=redacted",
        )]);
        let failure = CodexAdapter.plan(&context, &runner).unwrap_err();
        assert_eq!(failure.code(), "codex.config_incompatible");
        assert!(!failure.to_string().contains("secret"));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn 短工具期限需要升级且不重建已经正确的服务器配置() {
        let directory = tempfile::tempdir().unwrap();
        let (context, _, _) = installed_codex(directory.path());
        let previous = json!({"name": SERVER_NAME, "transport": {"type": "stdio", "command": context.mcp_executable, "args": []}, "tool_timeout_sec": 150});
        assert_eq!(
            codex_plan(&context, Some(&previous)).action,
            ConfigurationAction::Replace
        );
        let mut updated = previous.clone();
        updated["tool_timeout_sec"] = json!(86400);
        let config = directory.path().join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            format!(
                "[mcp_servers.agent_room]\ncommand = {}\ntool_timeout_sec = 150\n",
                serde_json::to_string(context.mcp_executable.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();
        let runner = FakeRunner::with(vec![
            output(0, &json!([previous]).to_string(), ""),
            output(0, &json!([updated]).to_string(), ""),
        ]);
        let digest = codex_plan(&context, Some(&previous)).original_digest;
        assert!(
            CodexAdapter
                .apply(&context, &runner, &digest)
                .unwrap()
                .changed
        );
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(_, args)| args == &["mcp", "list", "--json"])
        );
        assert!(
            fs::read_to_string(&config)
                .unwrap()
                .contains("tool_timeout_sec = 86400")
        );
        updated["tool_timeout_sec"] = json!(172_800);
        assert_eq!(
            codex_plan(&context, Some(&updated)).action,
            ConfigurationAction::Unchanged
        );
    }

    #[test]
    fn invalid_override_never_executes_an_unrelated_command() {
        let directory = tempfile::tempdir().unwrap();
        let (mut context, _, _) = installed_codex(directory.path());
        context.codex_cli_path = Some(PathBuf::from("relative-codex.exe"));
        let runner = FakeRunner::default();
        assert_eq!(
            CodexAdapter.plan(&context, &runner).unwrap_err().code(),
            "codex.executable_invalid"
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn replacement_failure_does_not_first_remove_existing_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let (context, _, _) = installed_codex(directory.path());
        let previous = json!({"name": SERVER_NAME, "transport": {"type": "stdio", "command": "old", "args": []}});
        let runner = FakeRunner::with(vec![
            output(0, &json!([previous]).to_string(), ""),
            output(1, "", "write denied"),
        ]);
        let failure = CodexAdapter
            .apply(&context, &runner, &digest_value(&previous))
            .unwrap_err();
        assert_eq!(failure.code(), "codex.add_failed");
        assert!(
            !runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(_, args)| args.iter().any(|value| value == "remove"))
        );
    }
}
