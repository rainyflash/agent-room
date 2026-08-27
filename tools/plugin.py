#!/usr/bin/env python3
"""构建、装配并校验 Agent Room Codex 插件发行归档。"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
PLUGIN_SOURCE = REPOSITORY_ROOT / "plugins" / "agent-room"
ARTIFACTS_ROOT = REPOSITORY_ROOT / "artifacts" / "codex-plugin"
BINARY_BASENAME = "agent-room-mcp"
MARKETPLACE_NAME = "agent-room-community"
PLUGIN_SELECTOR = f"agent-room@{MARKETPLACE_NAME}"
EXPECTED_TOOLS = (
    "agent_room_get_self",
    "agent_room_list_previews",
    "agent_room_get_presence",
    "agent_room_open_content",
    "agent_room_publish_status",
    "agent_room_send_message",
    "agent_room_consume_handoff",
    "agent_room_decline_handoff",
)
AUTOMATIC_TOOLS = (
    "agent_room_get_self",
    "agent_room_list_previews",
    "agent_room_get_presence",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=("stage", "validate", "host-check"),
        help="stage 构建发行归档；validate 校验源模板；host-check 在隔离 Codex 中安装验收",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        help="使用已有 MCP 二进制，省略时构建当前平台 release 版本",
    )
    parser.add_argument(
        "--platform-tag",
        help="覆盖归档平台标签；交叉编译流水线应显式传入",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    validate_source()
    if args.command == "validate":
        print(f"插件源模板校验通过：{PLUGIN_SOURCE}")
        return

    binary = resolve_binary(args.binary)
    platform_tag = args.platform_tag or current_platform_tag()
    stage_directory, archive = stage_plugin(binary, platform_tag)
    print(f"插件目录：{stage_directory}")
    print(f"插件归档：{archive}")
    if args.command == "host-check":
        smoke_test_codex_host(stage_directory)
        print("Codex 隔离宿主验证通过：两个独立进程均发现 agent_room")


def workspace_version() -> str:
    cargo_manifest = tomllib.loads(
        (REPOSITORY_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    )
    version = cargo_manifest.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str) or not version:
        raise RuntimeError("Cargo workspace 缺少有效版本")
    return version


def validate_source() -> None:
    manifest_path = PLUGIN_SOURCE / ".codex-plugin" / "plugin.json"
    mcp_path = PLUGIN_SOURCE / ".mcp.json"
    skill_path = PLUGIN_SOURCE / "skills" / "agent-room" / "SKILL.md"
    approval_policy_path = PLUGIN_SOURCE / "approval-policy.example.toml"
    for required in (manifest_path, mcp_path, skill_path, approval_policy_path):
        if not required.is_file():
            raise RuntimeError(f"插件源模板缺少文件：{required}")

    manifest = read_json_object(manifest_path)
    mcp_manifest = read_json_object(mcp_path)
    version = workspace_version()
    if manifest.get("name") != "agent-room":
        raise RuntimeError("插件名称必须为 agent-room")
    if manifest.get("version") != version:
        raise RuntimeError(
            f"插件版本 {manifest.get('version')!r} 与 workspace {version!r} 不一致"
        )

    servers = mcp_manifest.get("mcpServers")
    if not isinstance(servers, dict) or set(servers) != {"agent_room"}:
        raise RuntimeError(".mcp.json 必须且只能声明 agent_room 服务")
    server = servers["agent_room"]
    if not isinstance(server, dict):
        raise RuntimeError("agent_room MCP 服务配置必须是对象")
    if server.get("command") != f"./bin/{BINARY_BASENAME}":
        raise RuntimeError("MCP command 必须指向插件内置的无扩展名原生二进制")
    validate_approval_policy(approval_policy_path)


def validate_approval_policy(path: Path) -> None:
    policy = tomllib.loads(path.read_text(encoding="utf-8"))
    plugins = policy.get("plugins")
    if not isinstance(plugins, dict):
        raise RuntimeError("审批策略缺少 plugins 配置")
    plugin = plugins.get(PLUGIN_SELECTOR)
    if not isinstance(plugin, dict):
        raise RuntimeError(f"审批策略缺少插件选择器 {PLUGIN_SELECTOR}")
    servers = plugin.get("mcp_servers")
    server = servers.get("agent_room") if isinstance(servers, dict) else None
    if not isinstance(server, dict):
        raise RuntimeError("审批策略缺少 agent_room 服务配置")
    if server.get("enabled") is not True:
        raise RuntimeError("审批策略必须显式启用 agent_room")
    if server.get("default_tools_approval_mode") != "prompt":
        raise RuntimeError("非白名单工具必须保持逐次审批")
    if server.get("enabled_tools") != list(EXPECTED_TOOLS):
        raise RuntimeError("审批策略必须只允许既定的八个工具")
    tools = server.get("tools")
    if not isinstance(tools, dict) or set(tools) != set(AUTOMATIC_TOOLS):
        raise RuntimeError("审批策略只能自动放行三个最小只读工具")
    for tool_name in AUTOMATIC_TOOLS:
        tool = tools.get(tool_name)
        if not isinstance(tool, dict) or tool.get("approval_mode") != "approve":
            raise RuntimeError(f"只读工具 {tool_name} 必须配置为自动放行")


def read_json_object(path: Path) -> dict[str, object]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise RuntimeError(f"JSON 根节点必须是对象：{path}")
    return payload


def resolve_binary(explicit: Path | None) -> Path:
    if explicit is not None:
        binary = explicit.expanduser().resolve()
    else:
        run_checked(
            [
                "cargo",
                "build",
                "--release",
                "--locked",
                "-p",
                "agent-room-mcp",
            ]
        )
        suffix = ".exe" if os.name == "nt" else ""
        binary = REPOSITORY_ROOT / "target" / "release" / f"{BINARY_BASENAME}{suffix}"
    if not binary.is_file():
        raise RuntimeError(f"MCP 二进制不存在：{binary}")
    return binary


def current_platform_tag() -> str:
    system_names = {"Windows": "windows", "Darwin": "macos", "Linux": "linux"}
    architecture_names = {
        "AMD64": "x64",
        "x86_64": "x64",
        "arm64": "arm64",
        "aarch64": "arm64",
    }
    system = system_names.get(platform.system())
    architecture = architecture_names.get(platform.machine())
    if system is None or architecture is None:
        raise RuntimeError(
            f"无法推导发行平台：{platform.system()} {platform.machine()}，请传 --platform-tag"
        )
    return f"{system}-{architecture}"


def stage_plugin(binary: Path, platform_tag: str) -> tuple[Path, Path]:
    version = workspace_version()
    package_name = f"agent-room-plugin-v{version}-{platform_tag}"
    package_root = (ARTIFACTS_ROOT / package_name).resolve()
    assert_artifact_target(package_root)

    if package_root.exists():
        shutil.rmtree(package_root)
    shutil.copytree(PLUGIN_SOURCE, package_root)

    packaged_binary = package_root / "bin" / BINARY_BASENAME
    packaged_binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, packaged_binary)
    packaged_binary.chmod(
        packaged_binary.stat().st_mode
        | stat.S_IXUSR
        | stat.S_IXGRP
        | stat.S_IXOTH
    )
    validate_staged_plugin(package_root)
    smoke_test_mcp(packaged_binary, package_root)

    archive = ARTIFACTS_ROOT / f"{package_name}.zip"
    archive.parent.mkdir(parents=True, exist_ok=True)
    if archive.exists():
        archive.unlink()
    write_reproducible_zip(package_root, archive)
    return package_root, archive


def validate_staged_plugin(package_root: Path) -> None:
    packaged_binary = package_root / "bin" / BINARY_BASENAME
    if not packaged_binary.is_file() or packaged_binary.stat().st_size == 0:
        raise RuntimeError("装配后的插件缺少 MCP 二进制")
    if not (package_root / ".codex-plugin" / "plugin.json").is_file():
        raise RuntimeError("装配后的插件缺少 plugin.json")
    if not (package_root / ".mcp.json").is_file():
        raise RuntimeError("装配后的插件缺少 .mcp.json")


def smoke_test_mcp(binary: Path, working_directory: Path) -> None:
    requests = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "agent-room-package-check", "version": "0.1.0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "agent_room_get_self", "arguments": {}},
        },
    ]
    wire_input = "".join(
        f"{json.dumps(request, separators=(',', ':'))}\n" for request in requests
    )
    try:
        completed = subprocess.run(
            [str(binary)],
            cwd=working_directory,
            input=wire_input,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=20,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("MCP 冒烟测试超时") from error
    if completed.returncode != 0:
        raise RuntimeError(
            f"MCP 冒烟测试进程失败：{completed.stderr.strip() or completed.returncode}"
        )

    responses = parse_json_lines(completed.stdout)
    by_id = {
        response["id"]: response
        for response in responses
        if isinstance(response.get("id"), int)
    }
    initialize = require_result(by_id, 1)
    instructions = initialize.get("instructions")
    if not isinstance(instructions, str) or not instructions.startswith("安全边界"):
        raise RuntimeError("MCP initialize 未在开头声明远端内容安全边界")

    tools_result = require_result(by_id, 2)
    tools = tools_result.get("tools")
    if not isinstance(tools, list):
        raise RuntimeError("MCP tools/list 未返回工具数组")
    expected_tools = set(EXPECTED_TOOLS)
    actual_tools = {
        tool.get("name")
        for tool in tools
        if isinstance(tool, dict)
    }
    if actual_tools != expected_tools:
        raise RuntimeError(
            f"MCP 工具集合不一致：缺少 {expected_tools - actual_tools}，多出 {actual_tools - expected_tools}"
        )
    validate_tool_annotations(tools)

    call_result = require_result(by_id, 3)
    if not isinstance(call_result.get("content"), list):
        raise RuntimeError("agent_room_get_self 未返回 MCP 内容数组")


def validate_tool_annotations(tools: list[object]) -> None:
    expected = {
        "agent_room_get_self": (True, False, True, False),
        "agent_room_list_previews": (True, False, True, True),
        "agent_room_get_presence": (True, False, True, True),
        "agent_room_open_content": (True, False, True, True),
        "agent_room_publish_status": (False, False, True, True),
        "agent_room_send_message": (False, False, False, True),
        "agent_room_consume_handoff": (False, True, False, True),
        "agent_room_decline_handoff": (False, True, False, True),
    }
    for tool in tools:
        if not isinstance(tool, dict):
            raise RuntimeError("MCP 工具定义必须是对象")
        name = tool.get("name")
        annotations = tool.get("annotations")
        if not isinstance(name, str) or not isinstance(annotations, dict):
            raise RuntimeError("MCP 工具缺少名称或风险提示")
        hints = (
            annotations.get("readOnlyHint"),
            annotations.get("destructiveHint"),
            annotations.get("idempotentHint"),
            annotations.get("openWorldHint"),
        )
        if hints != expected[name]:
            raise RuntimeError(f"MCP 工具 {name} 的风险提示与真实语义不一致")


def smoke_test_codex_host(package_root: Path) -> None:
    codex = resolve_codex_cli()
    with tempfile.TemporaryDirectory(prefix="agent-room-codex-host-") as temporary:
        test_root = Path(temporary).resolve()
        marketplace_root = test_root / "marketplace"
        plugin_target = marketplace_root / "plugins" / "agent-room"
        shutil.copytree(package_root, plugin_target)
        marketplace_path = marketplace_root / ".agents" / "plugins" / "marketplace.json"
        marketplace_path.parent.mkdir(parents=True)
        marketplace_path.write_text(
            json.dumps(marketplace_manifest(), ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )

        codex_home = test_root / "codex-home"
        codex_home.mkdir()
        environment = os.environ.copy()
        environment["CODEX_HOME"] = str(codex_home)

        run_codex(codex, ["plugin", "marketplace", "add", str(marketplace_root)], environment)
        listing = run_codex(
            codex,
            ["plugin", "list", "--marketplace", MARKETPLACE_NAME],
            environment,
        )
        if "agent-room" not in listing.stdout:
            raise RuntimeError("Codex 市场未列出 agent-room 插件")

        run_codex(codex, ["plugin", "add", PLUGIN_SELECTOR], environment)
        append_approval_policy(codex_home / "config.toml")
        first = list_codex_mcp_servers(codex, environment)
        second = list_codex_mcp_servers(codex, environment)
        first_server = require_agent_room_server(first)
        second_server = require_agent_room_server(second)
        if first_server != second_server:
            raise RuntimeError("两个独立 Codex 进程解析出的 agent_room 服务不一致")
        validate_installed_transport(first_server, codex_home)


def marketplace_manifest() -> dict[str, object]:
    return {
        "name": MARKETPLACE_NAME,
        "interface": {"displayName": "Agent Room Community"},
        "plugins": [
            {
                "name": "agent-room",
                "source": {"source": "local", "path": "./plugins/agent-room"},
                "policy": {
                    "installation": "AVAILABLE",
                    "authentication": "ON_INSTALL",
                },
                "category": "Productivity",
            }
        ],
    }


def resolve_codex_cli() -> Path:
    candidates = ("codex.cmd", "codex.exe", "codex") if os.name == "nt" else ("codex",)
    for candidate in candidates:
        resolved = shutil.which(candidate)
        if resolved is not None:
            return Path(resolved).absolute()
    raise RuntimeError("未找到 Codex CLI；无法执行真实宿主验证")


def run_codex(
    executable: Path,
    arguments: list[str],
    environment: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    command = codex_command(executable, arguments)
    completed = subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        env=environment,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=30,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"Codex 命令失败：{' '.join(arguments)}：{detail}")
    return completed


def codex_command(executable: Path, arguments: list[str]) -> list[str]:
    if executable.suffix.lower() in {".cmd", ".bat"}:
        command_shell = os.environ.get("COMSPEC", "cmd.exe")
        return [command_shell, "/d", "/s", "/c", str(executable), *arguments]
    return [str(executable), *arguments]


def append_approval_policy(config_path: Path) -> None:
    existing = config_path.read_text(encoding="utf-8") if config_path.is_file() else ""
    policy = (PLUGIN_SOURCE / "approval-policy.example.toml").read_text(encoding="utf-8")
    separator = "" if not existing or existing.endswith("\n") else "\n"
    config_path.write_text(f"{existing}{separator}\n{policy}", encoding="utf-8")


def list_codex_mcp_servers(
    executable: Path, environment: dict[str, str]
) -> list[dict[str, object]]:
    completed = run_codex(executable, ["mcp", "list", "--json"], environment)
    payload = json.loads(completed.stdout)
    if not isinstance(payload, list) or not all(isinstance(item, dict) for item in payload):
        raise RuntimeError("Codex mcp list 未返回服务对象数组")
    return payload


def require_agent_room_server(
    servers: list[dict[str, object]],
) -> dict[str, object]:
    matches = [server for server in servers if server.get("name") == "agent_room"]
    if len(matches) != 1:
        raise RuntimeError(f"Codex 应且只应发现一个 agent_room，实际为 {len(matches)}")
    server = matches[0]
    if server.get("enabled") is not True:
        raise RuntimeError("Codex 中的 agent_room 服务未启用")
    return server


def validate_installed_transport(server: dict[str, object], codex_home: Path) -> None:
    transport = server.get("transport")
    if not isinstance(transport, dict) or transport.get("type") != "stdio":
        raise RuntimeError("Codex 未将 agent_room 注册为 STDIO 服务")
    command = transport.get("command")
    cwd = transport.get("cwd")
    if command != f"./bin/{BINARY_BASENAME}" or not isinstance(cwd, str):
        raise RuntimeError("Codex 未保留插件内置 MCP 的相对命令与工作目录")
    installed_root = Path(cwd).resolve()
    if codex_home.resolve() not in installed_root.parents:
        raise RuntimeError("Codex 未从隔离插件缓存加载 agent_room")
    if not (installed_root / "bin" / BINARY_BASENAME).is_file():
        raise RuntimeError("Codex 插件缓存缺少 agent_room MCP 二进制")
    if not (installed_root / ".mcp.json").is_file():
        raise RuntimeError("Codex 插件缓存缺少 .mcp.json")
    if not (installed_root / ".codex-plugin" / "plugin.json").is_file():
        raise RuntimeError("Codex 插件缓存缺少 plugin.json")


def parse_json_lines(output: str) -> list[dict[str, object]]:
    responses: list[dict[str, object]] = []
    for line in output.splitlines():
        if not line.strip():
            continue
        payload = json.loads(line)
        if not isinstance(payload, dict):
            raise RuntimeError("MCP STDIO 输出包含非对象 JSON")
        responses.append(payload)
    return responses


def require_result(
    responses: dict[int, dict[str, object]], request_id: int
) -> dict[str, object]:
    response = responses.get(request_id)
    if response is None:
        raise RuntimeError(f"MCP 缺少请求 {request_id} 的响应")
    result = response.get("result")
    if not isinstance(result, dict):
        raise RuntimeError(f"MCP 请求 {request_id} 未返回对象结果：{response}")
    return result


def assert_artifact_target(path: Path) -> None:
    artifacts = ARTIFACTS_ROOT.resolve()
    if path == artifacts or artifacts not in path.parents:
        raise RuntimeError(f"拒绝清理非插件产物目录：{path}")


def write_reproducible_zip(package_root: Path, archive: Path) -> None:
    with zipfile.ZipFile(
        archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as bundle:
        for source in sorted(package_root.rglob("*")):
            if not source.is_file():
                continue
            relative = Path(package_root.name) / source.relative_to(package_root)
            info = zipfile.ZipInfo(relative.as_posix(), date_time=(1980, 1, 1, 0, 0, 0))
            mode = source.stat().st_mode & 0o777
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(info, source.read_bytes())


def run_checked(command: list[str]) -> None:
    subprocess.run(command, cwd=REPOSITORY_ROOT, check=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"插件构建失败：{error}", file=sys.stderr)
        raise SystemExit(1) from error
