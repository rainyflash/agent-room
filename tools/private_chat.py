#!/usr/bin/env python3
"""用隔离服务验证原生 Agent 与浏览器无需核对即可加密私聊，以及重连。"""

from __future__ import annotations

import json
import os
import time

if __package__:
    from . import vertical as v
    from .mcp_client import McpAgentSession
else:
    import vertical as v
    from mcp_client import McpAgentSession


WORK = v.ROOT / "artifacts" / "private-chat"


def write(name: str, value: dict[str, object]) -> None:
    temporary = (WORK / name).with_suffix(".pending")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    temporary.replace(WORK / name)


def wait_file(name: str, browser: v.ManagedProcess) -> dict[str, object]:
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if (WORK / name).is_file():
            return v.require_object(json.loads((WORK / name).read_text(encoding="utf-8")), name)
        browser.ensure_running()
        time.sleep(0.4)
    raise v.VerticalFailure(f"浏览器未在期限内完成 {name}。")


def security(client: McpAgentSession, request: dict[str, object]) -> dict[str, object]:
    return v.require_object(
        client.call_tool("agent_room_matrix_security", {"request": request}).get("security"),
        "Matrix 安全状态",
    )


def roundtrip(
    client: McpAgentSession, browser: v.ManagedProcess, scenario: dict[str, object], phase: str
) -> None:
    sent = wait_file(f"{phase}-sent.json", browser)
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        result = client.call_tool("agent_room_list_previews", {"roomId": sent["roomId"], "limit": 30})
        previews = result.get("previews")
        if not isinstance(previews, list):
            raise v.VerticalFailure("MCP 预览结构无效。")
        preview = next((item for item in previews if isinstance(item, dict)
                        and item.get("messageId") == sent["messageId"]), None)
        if preview is not None:
            conversation = v.require_object(preview.get("conversation"), "已解密对话")
            actor = v.require_object(preview.get("actor"), "对话作者")
            if conversation.get("text") != scenario[f"{phase}Text"] or actor.get("kind") != "human":
                raise v.VerticalFailure("Agent 未读取到正确的人类私聊正文。")
            break
        browser.ensure_running()
        time.sleep(0.4)
    else:
        raise v.VerticalFailure("Agent 未在期限内读到加密私聊。")
    result = client.call_tool("agent_room_send_message", {
        "roomId": sent["roomId"], "submissionId": v.new_uuid_v7(), "chat": True,
        "body": scenario[f"{phase}Reply"], "provenance": "human_confirmed_agent",
        "replyToMessageId": sent["messageId"],
    })
    if v.require_object(result.get("message"), "私聊回复").get("state") != "submitted":
        raise v.VerticalFailure("Agent 私聊未提交。")
    write(f"{phase}-replied.json", {"submitted": True})


def main() -> None:
    v.configure_console_encoding()
    WORK.mkdir(parents=True, exist_ok=True)
    for name in ("peer.json", "first-sent.json", "first-replied.json", "restart-request.json",
                 "restarted.json", "second-sent.json", "second-replied.json", "done.json",
                 "result.json"):
        (WORK / name).unlink(missing_ok=True)
    environment = v.prepare_environment()
    redactor = v.LogRedactor(environment)
    outcome: dict[str, object] = {}
    v.build_runtime_binaries()
    with v.IsolatedInfrastructure():
        v.initialize_isolated_dependencies()
        catalog = v.seed_public_catalog()
        with v.IsolatedBridgeState(), v.ProcessStack() as processes:
            v.start_control_plane(processes, environment, redactor, WORK / "services")
            v.start_web(processes, redactor, WORK / "services")
            bootstrap = v.bootstrap_agent(environment)
            target = v.start_authorized_bridge(
                processes=processes, environment=environment, catalog_id=catalog,
                agent_id=bootstrap["agentId"], runtime_name="bridge-target",
                data_root=v.TARGET_BRIDGE_DATA_ROOT,
                secure_storage_service=v.TARGET_SECURE_STORAGE_SERVICE, redactor=redactor,
            )
            identity = v.require_bridge_session(target)
            tag = v.new_uuid_v7()
            scenario: dict[str, object] = {
                "catalogId": catalog, "targetName": target.display_name,
                "targetMatrixUserId": identity["agentMatrixUserId"],
                "publicRoomId": identity["matrixRoomId"],
                "firstText": f"Private human question {tag}", "firstReply": f"Private Agent reply {tag}",
                "secondText": f"Private question after restart {tag}",
                "secondReply": f"Private reply after restart {tag}",
            }
            write("input.json", scenario)
            browser_env = os.environ.copy()
            browser_env["AGENT_ROOM_PRIVATE_CHAT_PASSWORD"] = v.required_value(environment, "SEED_ADMIN_PASSWORD")
            try:
                with v.bridge_mcp_client(target, redactor) as transport:
                    client = transport.bind_session(identity["sessionId"])
                    # Bridge 上线时自动建立加密身份，不需要任何人手动操作；重复建立不改变身份。
                    prepared = security(client, {"action": "inspect"})
                    if prepared.get("state") != "ready":
                        raise v.VerticalFailure("Agent 上线后应已自动建立加密身份。")
                    if security(client, {"action": "establish_identity"}) != prepared:
                        raise v.VerticalFailure("重复建立改变了加密身份。")
                    browser = processes.start(v.ManagedProcess(
                        name="private-chat-browser",
                        command=[v.executable("node"), str(v.ROOT / "apps/web/node_modules/@playwright/test/cli.js"),
                                 "test", "--config", "apps/web/playwright.private-chat.config.ts"],
                        environment=browser_env, log_path=WORK / "services/browser.log", redactor=redactor,
                    ))
                    wait_file("peer.json", browser)
                    # 不核对安全码也能双向收发：人的设备登录后自动由主人签名（ADR 0011），
                    # 首次见到的身份被记住。
                    roundtrip(client, browser, scenario, "first")
                    print("Encrypted private chat worked in both directions without any verification.",
                          flush=True)
                    wait_file("restart-request.json", browser)
                v.close_bridge_session(target, redactor)
                generation = target.observation.agent_online_generation
                target.process.stop()
                target.process.start()
                target.observation.wait_for_agent_online(target.process, after_generation=generation, timeout_seconds=180)
                v.open_bridge_session(target, redactor)
                with v.bridge_mcp_client(target, redactor) as transport:
                    client = transport.bind_session(v.require_bridge_session(target)["sessionId"])
                    if security(client, {"action": "inspect"}) != prepared:
                        raise v.VerticalFailure("重启后加密身份或设备发生变化。")
                    write("restarted.json", {"ready": True})
                    roundtrip(client, browser, scenario, "second")
                    outcome.update(wait_file("done.json", browser))
                    if browser.process.wait(timeout=30) != 0:
                        raise v.VerticalFailure("浏览器私聊验收失败。")
                outcome["bridgeIdentityRestored"] = True
            finally:
                v.close_bridge_session(target, redactor)
    write("result.json", outcome)
    print(json.dumps(outcome, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
