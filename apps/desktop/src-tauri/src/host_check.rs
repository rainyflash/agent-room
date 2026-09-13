//! Read-only acceptance entrypoint for the same adapter used by the desktop UI.
use std::{path::Path, process::ExitCode};

use agent_room_host_adapters::{ConfigurationAction, HostConfigurator, HostContext, HostKind};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostCheck {
    host: HostKind,
    installed: bool,
    action: Option<ConfigurationAction>,
    error_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostCheckReport {
    schema_version: u8,
    version: &'static str,
    ok: bool,
    checks: Vec<HostCheck>,
}

pub(crate) fn run() -> ExitCode {
    let result = std::env::current_exe()
        .map_err(|_| "desktop.hosts.executable_unavailable".to_owned())
        .and_then(|executable| check_installed_hosts(&executable));
    match result {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => {
                println!("{json}");
                if report.ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(_) => ExitCode::FAILURE,
        },
        Err(code) => {
            eprintln!("Host checks failed [{code}]");
            ExitCode::FAILURE
        }
    }
}

fn check_installed_hosts(executable: &Path) -> Result<HostCheckReport, String> {
    let directory = executable
        .parent()
        .ok_or("desktop.hosts.directory_unavailable")?;
    let mcp = directory.join(format!("agent-room-mcp{}", std::env::consts::EXE_SUFFIX));
    let context = HostContext::from_environment(mcp).map_err(|error| error.code().to_owned())?;
    let configurator = HostConfigurator::system(context);
    let checks = configurator
        .detect_all()
        .into_iter()
        .map(|detection| {
            let mut check = HostCheck {
                host: detection.host,
                installed: detection.installed,
                action: None,
                error_code: None,
            };
            if detection.installed {
                match configurator.plan(detection.host) {
                    Ok(plan) => check.action = Some(plan.action),
                    Err(error) => check.error_code = Some(error.code().to_owned()),
                }
            }
            check
        })
        .collect::<Vec<_>>();
    Ok(HostCheckReport {
        schema_version: 1,
        version: env!("CARGO_PKG_VERSION"),
        ok: checks.iter().all(|check| check.error_code.is_none()),
        checks,
    })
}
