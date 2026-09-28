#![allow(clippy::missing_errors_doc)]

//! 后台回复按宿主恢复已登记的任务，这里负责在本机找到能用的宿主程序；
//! 另外给出通用 MCP 配置里的服务名和命令。不读、也不改任何宿主自己的配置。

use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

const SERVER_NAME: &str = "agent_room";
mod command;

pub use command::SystemCommandRunner;

/// 能替后台回复恢复任务的宿主。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostKind {
    Codex,
    ClaudeCode,
}

/// 任何支持 MCP 的工具都照这份配置添加本机的 Agent Room 服务。
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
    pub path_entries: Vec<PathBuf>,
    pub mcp_executable: PathBuf,
    /// `CODEX_CLI_PATH`：明确指定的 Codex 程序，设了就只用它。
    pub codex_cli_path: Option<PathBuf>,
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
            path_entries,
            mcp_executable,
            codex_cli_path: env::var_os("CODEX_CLI_PATH").map(PathBuf::from),
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("宿主程序不可用：{code}")]
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

pub struct HostConfigurator {
    context: HostContext,
    runner: Arc<dyn CommandRunner>,
}

impl HostConfigurator {
    pub fn system(context: HostContext) -> Self {
        Self::new(context, Arc::new(SystemCommandRunner))
    }

    pub fn new(context: HostContext, runner: Arc<dyn CommandRunner>) -> Self {
        Self { context, runner }
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
            HostKind::Codex => compatible_codex(&self.context, self.runner.as_ref())?,
            HostKind::ClaudeCode => path_commands(&self.context, "claude")
                .into_iter()
                .find(|path| {
                    !cfg!(windows)
                        || path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                })
                .ok_or_else(|| HostFailure::new("claude.not_installed", false))?,
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
}

/// 候选的 Codex：明确指定的只用它；否则先用桌面版自带的（新装的在前），再用 PATH 里的。
fn codex_candidates(context: &HostContext) -> Vec<PathBuf> {
    if let Some(path) = &context.codex_cli_path {
        // An explicit override must not silently fall back to another installation.
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

/// 第一个能正常读取自己设置的 Codex。读不懂当前设置的旧版本跳过，换下一个候选。
fn compatible_codex(
    context: &HostContext,
    runner: &dyn CommandRunner,
) -> Result<PathBuf, HostFailure> {
    let mut failure = None;
    for executable in codex_candidates(context) {
        if !executable.is_absolute() || !executable.is_file() {
            return Err(HostFailure::new("codex.executable_invalid", false));
        }
        match codex_state(runner, &executable) {
            Ok(()) => return Ok(executable),
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    Err(failure.unwrap_or_else(|| HostFailure::new("codex.not_installed", false)))
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

/// 让这个 Codex 列一次 MCP 服务器，看它能不能读懂自己的设置。只读，不改任何配置；
/// 失败按原因分类，不把命令输出（可能含设置内容）带进错误。
fn codex_state(runner: &dyn CommandRunner, executable: &Path) -> Result<(), HostFailure> {
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
    serde_json::from_str::<Vec<serde::de::IgnoredAny>>(&output.stdout)
        .map(|_| ())
        .map_err(|_| HostFailure::new("codex.list_invalid", false))
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
        fn with(outputs: Vec<CommandOutput>) -> Arc<Self> {
            Arc::new(Self {
                outputs: Mutex::new(outputs.into()),
                calls: Mutex::default(),
            })
        }

        fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.calls.lock().expect("测试锁不能中毒").clone()
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
                .expect("测试锁不能中毒")
                .push((executable.into(), arguments.to_vec()));
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
            path_entries: vec![],
            mcp_executable: mcp,
            codex_cli_path: None,
        }
    }

    fn output(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    /// 本平台的原生程序名：Windows 上带 `.exe`。
    fn native(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    }

    fn write_file(path: &Path) -> PathBuf {
        fs::create_dir_all(path.parent().expect("有父目录")).expect("目录可创建");
        fs::write(path, b"program").expect("程序文件可写");
        path.to_path_buf()
    }

    /// 桌面版 Codex 自带一份原生程序；PATH 里还有一份较旧的原生安装。
    fn installed_codex(root: &Path) -> (HostContext, PathBuf, PathBuf) {
        let mut context = context(root);
        let bundled = write_file(&root.join("OpenAI/Codex/bin/new-build/codex.exe"));
        let on_path = write_file(&root.join("path-bin").join(native("codex")));
        context.local_app_data = Some(root.into());
        context.path_entries.push(root.join("path-bin"));
        (context, bundled, on_path)
    }

    fn reception(
        context: HostContext,
        runner: &Arc<FakeRunner>,
        host: HostKind,
    ) -> Result<PathBuf, HostFailure> {
        HostConfigurator::new(context, runner.clone()).reception_executable(host)
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

    #[test]
    fn 后台回复优先用桌面版自带的_codex_只读取不改配置() {
        let directory = tempfile::tempdir().unwrap();
        let (context, bundled, _) = installed_codex(directory.path());
        let runner = FakeRunner::with(vec![output(0, "[]", "")]);
        assert_eq!(
            reception(context, &runner, HostKind::Codex).unwrap(),
            bundled
        );
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, bundled);
        assert_eq!(calls[0].1, ["mcp", "list", "--json"]);
    }

    #[test]
    fn 读不懂设置的_codex_跳过_改用下一个能用的安装() {
        let directory = tempfile::tempdir().unwrap();
        let (context, bundled, on_path) = installed_codex(directory.path());
        let runner = FakeRunner::with(vec![
            output(1, "", "unknown variant `max` secret-not-for-ui"),
            output(0, "[]", ""),
        ]);
        assert_eq!(
            reception(context, &runner, HostKind::Codex).unwrap(),
            on_path
        );
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, bundled);
        assert_eq!(calls[1].0, on_path);
        assert!(
            calls
                .iter()
                .all(|(_, args)| args == &["mcp", "list", "--json"])
        );
    }

    #[test]
    fn 所有候选都不能用时报第一个原因且不泄露设置内容() {
        let directory = tempfile::tempdir().unwrap();
        let (context, _, _) = installed_codex(directory.path());
        let runner = FakeRunner::with(vec![
            output(1, "", "Error loading config: secret=redacted"),
            output(1, "", "network down"),
        ]);
        let failure = reception(context, &runner, HostKind::Codex).unwrap_err();
        assert_eq!(failure.code(), "codex.config_invalid");
        assert!(failure.retryable());
        assert!(!failure.to_string().contains("secret"));
    }

    #[test]
    fn 明确指定的_codex_只用它_失败按原因分类() {
        let directory = tempfile::tempdir().unwrap();
        let (mut context, _, on_path) = installed_codex(directory.path());
        context.codex_cli_path = Some(on_path.clone());
        let runner = FakeRunner::with(vec![output(
            1,
            "",
            "failed to load configuration: unknown variant `max`; secret=redacted",
        )]);
        let failure = reception(context.clone(), &runner, HostKind::Codex).unwrap_err();
        assert_eq!(failure.code(), "codex.config_incompatible");
        assert!(!failure.to_string().contains("secret"));
        // 不退回桌面版自带的那份。
        assert_eq!(runner.calls().len(), 1);
        assert_eq!(runner.calls()[0].0, on_path);

        let runner = FakeRunner::with(vec![output(0, "[]", "")]);
        assert_eq!(
            reception(context, &runner, HostKind::Codex).unwrap(),
            on_path
        );
    }

    #[test]
    fn 指定的_codex_不是绝对路径时不执行任何命令() {
        let directory = tempfile::tempdir().unwrap();
        let (mut context, _, _) = installed_codex(directory.path());
        context.codex_cli_path = Some(PathBuf::from("relative-codex.exe"));
        let runner = FakeRunner::with(vec![]);
        assert_eq!(
            reception(context, &runner, HostKind::Codex)
                .unwrap_err()
                .code(),
            "codex.executable_invalid"
        );
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn codex_列表失败或输出无效都有明确的诊断码() {
        let directory = tempfile::tempdir().unwrap();
        let (mut context, _, on_path) = installed_codex(directory.path());
        context.codex_cli_path = Some(on_path);
        for (result, code) in [
            (output(1, "", "permission denied"), "codex.list_failed"),
            (output(0, "not json", ""), "codex.list_invalid"),
        ] {
            let runner = FakeRunner::with(vec![result]);
            assert_eq!(
                reception(context.clone(), &runner, HostKind::Codex)
                    .unwrap_err()
                    .code(),
                code
            );
        }
        let runner = FakeRunner::with(vec![]);
        assert_eq!(
            reception(self::context(directory.path()), &runner, HostKind::Codex)
                .unwrap_err()
                .code(),
            "codex.not_installed"
        );
    }

    #[test]
    fn 只能经命令脚本启动的_codex_不能用于后台回复() {
        let directory = tempfile::tempdir().unwrap();
        let mut context = context(directory.path());
        let wrapper = write_file(&directory.path().join("npm").join(if cfg!(windows) {
            "codex.cmd"
        } else {
            "codex"
        }));
        context.path_entries.push(directory.path().join("npm"));
        let runner = FakeRunner::with(vec![output(0, "[]", "")]);
        let result = reception(context, &runner, HostKind::Codex);
        if cfg!(windows) {
            assert_eq!(
                result.unwrap_err().code(),
                "host.native_executable_required"
            );
        } else {
            assert_eq!(result.unwrap(), wrapper);
        }
    }

    #[test]
    fn claude_code_用_path_里的原生程序_跳过命令脚本() {
        let directory = tempfile::tempdir().unwrap();
        let mut context = context(directory.path());
        let runner = FakeRunner::with(vec![]);
        assert_eq!(
            reception(context.clone(), &runner, HostKind::ClaudeCode)
                .unwrap_err()
                .code(),
            "claude.not_installed"
        );
        if cfg!(windows) {
            // 排在前面的 npm 脚本不算：后台回复要直接启动原生程序。
            write_file(&directory.path().join("npm/claude.cmd"));
            context.path_entries.push(directory.path().join("npm"));
            assert_eq!(
                reception(context.clone(), &runner, HostKind::ClaudeCode)
                    .unwrap_err()
                    .code(),
                "claude.not_installed"
            );
        }
        let installed = write_file(&directory.path().join("bin").join(native("claude")));
        context.path_entries.push(directory.path().join("bin"));
        assert_eq!(
            reception(context, &runner, HostKind::ClaudeCode).unwrap(),
            installed
        );
        // 找 Claude Code 不用运行任何命令。
        assert!(runner.calls().is_empty());
    }
}
