use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{CommandOutput, CommandRunner, HostFailure};

const MAX_OUTPUT: u64 = 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, executable: &Path, arguments: &[String]) -> Result<CommandOutput, HostFailure> {
        run_command(executable, arguments, COMMAND_TIMEOUT)
    }
}

fn run_command(
    executable: &Path,
    arguments: &[String],
    timeout: Duration,
) -> Result<CommandOutput, HostFailure> {
    let mut child = platform_command(executable, arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| HostFailure::new("host.command_spawn_failed", true))?;
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    for (is_stdout, stream) in [
        (true, Box::new(stdout) as Box<dyn Read + Send>),
        (false, Box::new(stderr)),
    ] {
        let sender = sender.clone();
        thread::spawn(move || {
            let output = read_output(stream);
            let _ = sender.send((is_stdout, output));
        });
    }
    drop(sender);
    let started = Instant::now();
    let result = (|| {
        let (mut stdout, mut stderr, mut status) = (None, None, None);
        loop {
            while let Ok((is_stdout, output)) = receiver.try_recv() {
                if is_stdout {
                    stdout = Some(output?);
                } else {
                    stderr = Some(output?);
                }
            }
            if status.is_none() {
                status = child
                    .try_wait()
                    .map_err(|_| HostFailure::new("host.command_wait_failed", true))?;
            }
            if let (Some(status), Some(stdout), Some(stderr)) =
                (status, stdout.as_ref(), stderr.as_ref())
            {
                return Ok(CommandOutput {
                    status: status.code().unwrap_or(-1),
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                });
            }
            if started.elapsed() >= timeout {
                return Err(HostFailure::new("host.command_timed_out", true));
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        stop_command(&mut child);
    }
    result
}

fn read_output(stream: impl Read) -> Result<String, HostFailure> {
    let mut bytes = Vec::new();
    stream
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| HostFailure::new("host.command_output_unavailable", true))?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(HostFailure::new("host.command_output_too_large", false));
    }
    String::from_utf8(bytes).map_err(|_| HostFailure::new("host.command_output_invalid", false))
}

fn stop_command(child: &mut Child) {
    // npm wrappers launch descendants. Killing only cmd.exe leaves the actual
    // configuration command running after the UI has reported a timeout.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn platform_command(executable: &Path, arguments: &[String]) -> Command {
    use std::os::windows::process::CommandExt as _;
    let extension = executable.extension().and_then(std::ffi::OsStr::to_str);
    let mut command = if matches!(extension, Some("cmd" | "bat")) {
        let mut value = Command::new("cmd.exe");
        value.args(["/d", "/s", "/c"]).arg(executable);
        value
    } else {
        Command::new(executable)
    };
    command.args(arguments).creation_flags(0x0800_0000);
    command
}

#[cfg(not(windows))]
fn platform_command(executable: &Path, arguments: &[String]) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn output_is_bounded_before_allocation_can_grow_indefinitely() {
        let stream = io::repeat(b'x').take(MAX_OUTPUT + 5);
        assert_eq!(
            read_output(stream).unwrap_err().code(),
            "host.command_output_too_large"
        );
    }

    #[test]
    fn command_errors_keep_stderr_for_classification() {
        #[cfg(windows)]
        let (exe, args) = (
            "cmd.exe",
            vec![
                "/d".into(),
                "/c".into(),
                "echo invalid-config 1>&2 & exit /b 1".into(),
            ],
        );
        #[cfg(not(windows))]
        let (exe, args) = (
            "sh",
            vec!["-c".into(), "echo invalid-config >&2; exit 1".into()],
        );
        let output = run_command(Path::new(exe), &args, Duration::from_secs(5)).unwrap();
        assert_eq!(output.status, 1);
        assert!(output.stderr.contains("invalid-config"));
    }

    #[test]
    fn hung_host_command_returns_a_retryable_timeout() {
        #[cfg(windows)]
        let (exe, args) = (
            "cmd.exe",
            vec!["/d".into(), "/c".into(), "ping -n 10 127.0.0.1 >nul".into()],
        );
        #[cfg(not(windows))]
        let (exe, args) = ("sleep", vec!["5".into()]);
        let failure = run_command(Path::new(exe), &args, Duration::from_millis(100)).unwrap_err();
        assert_eq!(failure.code(), "host.command_timed_out");
        assert!(failure.retryable());
    }
}
