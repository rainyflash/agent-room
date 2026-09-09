"""Verify Linux vault login, restart identity, Matrix delivery and MCP without an OS keyring.

Uses only the repository's isolated local test infrastructure and seeded accounts.
It never claims to have run an authenticated external model host.
"""
from __future__ import annotations

import json
import sys

import vertical as v


def restart(
    runtime: v.AuthorizedBridgeRuntime,
    processes: v.ProcessStack,
    redactor: v.LogRedactor,
    *,
    crash: bool = False,
) -> None:
    v.close_bridge_session(runtime, redactor)
    previous = runtime.process.process
    if crash:
        previous.kill()  # Only this acceptance harness's owned Bridge process.
        previous.wait(timeout=10)
    runtime.process.stop()
    if not crash and previous.returncode != 0:
        raise v.VerticalFailure("Bridge did not shut down gracefully on SIGTERM.")
    observation = v.BridgeRuntimeObservation()
    name = "bridge-target-crash-restored" if crash else "bridge-target-restored"
    process = processes.start(v.ManagedProcess(
        name=name, command=[str(v.runtime_binary("agent-room-bridge"))],
        environment=runtime.environment, log_path=v.LOG_ROOT / f"{name}.log",
        redactor=redactor, on_line=observation.observe,
    ))
    # No new device grant approval. Reaching online must use the encrypted saved credentials.
    observation.wait_for_agent_online(process, timeout_seconds=180)
    runtime.process = process
    runtime.observation = observation
    v.open_bridge_session(runtime, redactor)  # checks exact Agent, instance, device and room identity


def accept() -> None:
    if not sys.platform.startswith("linux"):
        raise v.VerticalFailure("Headless acceptance requires an isolated Linux runner.")
    environment = v.prepare_environment()
    v.build_runtime_binaries(("agent-room-control-plane", "agent-room-bridge", "agent-room-mcp", "agent-room-cli"))
    with v.IsolatedInfrastructure():
        v.initialize_isolated_dependencies()
        catalog = v.seed_public_catalog()
        redactor = v.LogRedactor(environment)
        with v.IsolatedBridgeState(vault=True), v.ProcessStack() as processes:
            v.start_control_plane(processes, environment, redactor)
            v.start_web(processes, redactor)
            agent = v.bootstrap_agent(environment)
            runtimes = []
            for name, data_root, service in zip(
                ("bridge-sender", "bridge-target"), v.BRIDGE_DATA_ROOTS, v.SECURE_STORAGE_SERVICES, strict=True
            ):
                runtimes.append(v.start_authorized_bridge(processes=processes, environment=environment,
                    catalog_id=catalog, agent_id=agent["agentId"], runtime_name=name, data_root=data_root,
                    secure_storage_service=service, redactor=redactor, vault=True))
            sender, target = runtimes
            identity = dict(v.require_bridge_session(target))
            restart(target, processes, redactor)
            restart(target, processes, redactor, crash=True)
            recovery = v.verify_matrix_disconnect_and_recovery(target, (sender,), redactor)
            result = v.verify_mcp_workflow(target_bridge=target, sender_bridge=sender,
                principal_id=agent["principalId"], redactor=redactor)
            if result["agentInstanceId"] != identity["agentInstanceId"]:
                raise v.VerticalFailure("Server restart changed the Agent instance.")
            v.run_checked([str(v.runtime_binary("agent-room")), "doctor"], environment=target.environment)
        logs = tuple(v.LOG_ROOT.glob("*.log"))
        v.verify_sanitized_logs(logs, redactor, additional_secrets=tuple(runtime.device_code for runtime in runtimes))
    report = v.ROOT / "artifacts" / "agent-runtime-live.json"
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(json.dumps({"platform":"linux", "vaultRestored":True, "identityPreserved":True,
        "gracefulRestart":True, "crashRecovered":True,
        "matrixRecoveryGeneration":recovery, "matrixDeliveryTested":True, "replyMessageId":result["replyMessageId"],
        "hostModelInvoked":False}, indent=2) + "\n", encoding="utf-8")
    print(report.read_text(encoding="utf-8"))


if __name__ == "__main__":
    v.configure_console_encoding()
    accept()
