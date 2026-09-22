#!/usr/bin/env python3
"""Produce the three same-candidate usability acceptances on the release workstation.

release_flow.py only *verifies* acceptance reports; this runner *produces* them from real
observations of the signed candidate:

* an isolated Bridge profile that the owner authorizes with a device code, or the recorded long-lived
  acceptance device restoring its saved authorization while the acceptance policy allows it (first-device);
* the owner's real host task receiving two real messages through Agent Room, one with a
  random attachment canary it can only read from the delivered attachment, then staying
  idle, being taken over and resumed (continuous-reception);
* the owner's actual previous installation upgraded in place by the verified installer,
  keeping its login, the upgrade test identity and its unacknowledged deliveries (upgrade).

It never approves the device code, never types credentials and never marks a check as
passed that it did not observe. The installed desktop app is driven over its local
WebView debugging port, which the runner opens for the run and closes again at cleanup.

Usage (Windows workstation):

    python tools/release_qa.py --work <release dir> baseline # before the server upgrade: pending deliveries
    python tools/release_qa.py --work <release dir> upgrade  # old install -> candidate, evidence
    python tools/release_qa.py --work <release dir> start    # isolated Bridge, prints the link
    python tools/release_qa.py --work <release dir> run      # after approval: everything else
    python tools/release_qa.py --work <release dir> cleanup  # revoke, leave, stop, close CDP
    python tools/release_qa.py --work <release dir> status

`run` resumes after interruption: every step records its result once and is skipped when
that record exists. Exit codes: 0 done, 20 waiting for the owner, 1 failure.

With `"device": {"record": "<path>"}` in `release-qa.json`, a fresh run records its authorized profile as
the long-lived acceptance device, and later releases reuse it without a device code for up to 30 days
unless login code changes (`release_acceptance.reuse_blocker`). `start` also creates the Claude Code host
session named in `qa-host.json` when it does not exist yet.

The release directory holds the verified candidate (`candidate/`, `verified-candidate.json`,
`ci-verification.json`) and the host description (`qa-host.json`). `upgrade` writes
`upgrade-baseline.json`, `installed-verification.json`, `native-session-restoration.json` and
`usability-evidence-upgrade.json` next to them. Deployment details live in `release-qa.json`:

    {"controlPlaneUrl": "https://api.example/", "matrixBaseUrl": "https://matrix.example/",
     "oidcIssuerUrl": "https://id.example/realms/agent-room", "oidcDeviceClientId": "agent-room-bridge",
     "room": {"roomId": "!room:example", "catalogId": "<UUIDv7>"},
     "desktop": {"executable": "C:/.../agent-room-desktop.exe", "origin": "http://tauri.localhost",
                 "cdpPort": 14222},
     "upgrade": {"identity": "<native-join.json of the long-lived upgrade identity>",
                 "installDir": "C:/Users/<user>/AppData/Local/Agent Room"}}
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import ntpath
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time
from typing import Any, Callable
import urllib.parse

try:
    from .release import ReleaseFailure
    from . import release_acceptance
except ImportError:
    from release import ReleaseFailure
    import release_acceptance


ROOT = Path(__file__).resolve().parents[1]
CDP_RUNNER = ROOT / "tools" / "cdp_run.mjs"
WAITING = 20
NEWLINE = chr(10)
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
UUID7 = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
HOST_NAMES = {"codex": "Codex", "claude_code": "Claude Code"}
IDLE_OBSERVATION_SECONDS = 66
# After a server move the owner registers and signs in on the new server while the runner waits.
MIGRATION_SIGN_IN_SECONDS = 20 * 60
# Accessible names of the room controls in both UI languages, so the desktop language does not matter.
LABELS = {
    "input": ("聊天消息", "Message"),
    "mention": ("提及 Agent", "Mention an agent"),
    "attach": ("添加附件", "Attach a file"),
    "send": ("发送", "Send"),
    "log": ("房间对话", "Conversation"),
}


def load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def js_name(key: str) -> str:
    """An exact-match RegExp literal for one control in either language (no /u: plain escapes)."""
    escaped = [re.sub(r"([\\^$.*+?()[\]{}|/])", r"\\\1", name) for name in LABELS[key]]
    return "/^(" + "|".join(escaped) + ")$/"


def release_label(version: str) -> str:
    """0.1.0-alpha.41 -> Alpha 41; other versions are shown as they are."""
    match = re.fullmatch(r"\d+\.\d+\.\d+-([a-z]+)\.(\d+)", version)
    return f"{match.group(1).capitalize()} {match.group(2)}" if match else version


def release_slug(version: str) -> str:
    """0.1.0-alpha.41 -> alpha41, used for the isolated credential namespace and file names."""
    return re.sub(r"[^a-z0-9]", "", version.rsplit("-", 1)[-1].lower())


class Acceptance:
    def __init__(self, work: Path) -> None:
        self.work = work.resolve()
        self.qa = self.work / "fresh-device-qa"
        self.candidate = self.work / "candidate"
        self.config = load(self.work / "release-qa.json")
        self.verified = load(self.work / "verified-candidate.json")
        self.metadata = load(self.candidate / "release-metadata.json")
        self.manifest = load(self.candidate / "release.json")
        self.host = load(self.work / "qa-host.json")
        self.version = self.metadata["version"]
        if self.verified["version"] != self.version or self.manifest["version"] != self.version:
            raise ReleaseFailure("候选、核验记录和清单的版本不一致。")
        if digest(self.candidate / "release.signed.json") != self.verified["signedManifestSha256"]:
            raise ReleaseFailure("签名清单与已核验候选不一致。")
        required = ["independentRootSignaturePassed", "sigstorePassed", "certificateRevisionPassed",
                    "tauriSignaturePassed", "ciInstallerAcceptanceMatched",
                    "artifactDigestsAndSbomPassed", "remoteIndexDigestsPassed"]
        if not all(self.verified.get(key) is True for key in required):
            raise ReleaseFailure("候选尚未通过全部独立核验。")
        if not self.host.get("realHostInitialized"):
            raise ReleaseFailure("真实宿主任务尚未初始化：先在验收工作区创建宿主会话。")
        self.host_type = self.host.get("hostType", "codex")
        if self.host_type not in HOST_NAMES:
            raise ReleaseFailure(f"不支持的宿主类型：{self.host_type}")
        self.label = release_label(self.version)
        self.slug = release_slug(self.version)
        device = self.config.get("device")
        self.device_record_path = Path(device["record"]) if isinstance(device, dict) else None
        self.use_fresh_device()
        self.canary_name = f"{self.slug}-attachment.txt"
        self.migration = release_acceptance.SERVER_MIGRATIONS.get(self.version)
        self.room_id = self.config["room"]["roomId"]
        self.catalog_id = self.config["room"]["catalogId"]
        self.api = self.config["controlPlaneUrl"].rstrip("/")
        desktop = self.config["desktop"]
        self.desktop_executable = ntpath.normpath(desktop["executable"])
        self.origin = desktop.get("origin", "http://tauri.localhost")
        self.cdp_port = int(desktop.get("cdpPort", 14222))
        if self.path("device-mode.json").exists():
            self.apply_device(load(self.path("device-mode.json")))

    # ------------------------------------------------------------------ candidate binaries

    def executable(self, name: str) -> str:
        assets = [a for a in self.manifest["artifacts"] if a["name"] == name and a["platform"] == "windows-x86_64"]
        if len(assets) != 1:
            raise ReleaseFailure(f"候选清单里没有唯一的 {name}。")
        path = (self.candidate / assets[0]["url"].rsplit("/", 1)[1]).resolve(strict=True)
        if path.parent != self.candidate.resolve() or digest(path) != assets[0]["sha256"]:
            raise ReleaseFailure(f"{name} 不是清单中的签名产物。")
        return str(path)

    def host_env(self) -> dict[str, str]:
        # Strip Agent Room overrides and any Claude Code session variables so the host child starts clean.
        env = {k: v for k, v in os.environ.items() if not k.startswith("AGENT_ROOM_") and not k.upper().startswith("CLAUDE")}
        if self.host_type == "codex":
            env["CODEX_THREAD_ID"] = self.host["taskId"]
        elif self.host.get("model"):
            env["ANTHROPIC_MODEL"] = self.host["model"]
        return env

    def bridge_env(self) -> dict[str, str]:
        return dict(
            self.host_env(),
            AGENT_ROOM_CONTROL_PLANE_URL=self.config["controlPlaneUrl"],
            AGENT_ROOM_MATRIX_BASE_URL=self.config["matrixBaseUrl"],
            AGENT_ROOM_OIDC_ISSUER_URL=self.config["oidcIssuerUrl"],
            AGENT_ROOM_OIDC_DEVICE_CLIENT_ID=self.config["oidcDeviceClientId"],
            AGENT_ROOM_BRIDGE_DATA_DIR=str(self.data),
            AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE=self.service,
            AGENT_ROOM_BRIDGE_DEVICE_LABEL=self.device_label,
            AGENT_ROOM_BRIDGE_SUPERVISED="true",
        )

    def cli(self, *args: str, timeout: int = 60) -> Any:
        command = [self.executable("agent-cli"), "--data-root", str(self.data), "--connection", self.service, *args]
        result = subprocess.run(command, env=self.host_env(), capture_output=True, encoding="utf-8",
                                timeout=timeout, creationflags=NO_WINDOW)
        try:
            envelope = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise ReleaseFailure(f"{args[0]} 没有返回 JSON：{result.stderr.strip()[-300:]}") from error
        if result.returncode or envelope.get("ok") is not True:
            raise ReleaseFailure(f"{args[-1] if args else 'cli'} 失败：{envelope.get('error')}")
        return envelope["data"]

    # ------------------------------------------------------------------ records

    def path(self, name: str) -> Path:
        return self.qa / name

    def save(self, name: str, value: Any, *, overwrite: bool = False) -> Path:
        path = self.qa / name
        mode = "w" if overwrite else "x"
        with path.open(mode, encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
        return path

    def background(self, args: list[str], name: str, env: dict[str, str]) -> dict[str, Any]:
        with self.path(f"{name}.private.jsonl").open("ab") as out, self.path(f"{name}.private.stderr").open("ab") as err:
            child = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=err, creationflags=NO_WINDOW)
        record = {"pid": child.pid, "startedAtUnixSeconds": int(time.time()), "executable": args[0]}
        self.save(f"{name}-process.json", record, overwrite=True)
        return record

    def alive(self, record_name: str) -> bool:
        record_path = self.path(record_name)
        if not record_path.exists():
            return False
        record = load(record_path)
        script = liveness_script(int(record["pid"]), record["executable"])
        return subprocess.check_output(["powershell", "-NoProfile", "-Command", script], encoding="utf-8",
                                       creationflags=NO_WINDOW).strip() == "yes"

    def stop(self, record_name: str) -> None:
        if not self.alive(record_name):
            return
        record = load(self.path(record_name))
        subprocess.run(["powershell", "-NoProfile", "-Command",
                        f"$p = Get-Process -Id {int(record['pid'])}; Stop-Process -Id $p.Id; $p.WaitForExit(10000) | Out-Null"],
                       check=True, creationflags=NO_WINDOW)

    def events(self, name: str) -> list[dict[str, Any]]:
        path = self.path(f"{name}.private.jsonl")
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.startswith("{")]

    # ------------------------------------------------------------------ first-device: acceptance device

    def use_fresh_device(self) -> None:
        # With a long-lived device configured, every fresh run creates the same stable character name.
        self.agent_name = (f"发布验收 {HOST_NAMES[self.host_type]}" if self.device_record_path is not None
                           else f"{self.label} 实机验收 {HOST_NAMES[self.host_type]}")
        self.service = f"agent-room.{self.slug}.acceptance.fresh-device"
        self.data = self.qa / "bridge-data"
        self.device_label = f"{self.label} fresh-device acceptance"

    def apply_device(self, decision: dict[str, Any]) -> None:
        if decision["mode"] != "reused":
            self.use_fresh_device()
            return
        record = decision["record"]
        self.data = Path(record["dataDir"])
        self.service = record["service"]
        self.device_label = record["label"]
        self.agent_name = record["agentName"]

    def device_mode(self) -> str:
        decision = self.path("device-mode.json")
        return str(load(decision)["mode"]) if decision.exists() else "fresh"

    def decide_device(self) -> dict[str, Any]:
        """Reuse the long-lived acceptance device when the acceptance policy allows it, else a fresh profile."""
        if self.path("device-mode.json").exists():
            return load(self.path("device-mode.json"))
        decision: dict[str, Any] = {"mode": "fresh"}
        if self.device_record_path is not None and self.device_record_path.exists():
            record = load(self.device_record_path)
            try:
                blocker = release_acceptance.reuse_blocker(record.get("freshAuthorization"),
                                                           self.metadata["revision"], int(time.time()))
            except ReleaseFailure as failure:
                blocker = str(failure)
            decision = {"mode": "reused", "record": record} if blocker is None else {"mode": "fresh", "reason": blocker}
        self.save("device-mode.json", decision)
        return decision

    def remember_device(self) -> None:
        """After a fresh run its authorized profile becomes the long-lived acceptance device."""
        if self.device_record_path is None or self.device_mode() != "fresh":
            return
        joined = self.joined()
        first = load(self.work / "usability-evidence-first-device.json")
        record = {"schemaVersion": 1, "dataDir": str(self.data.resolve()), "service": self.service,
                  "label": self.device_label, "profileId": joined["profileId"],
                  "agentId": joined["identity"]["agent"]["agentId"], "agentName": self.agent_name,
                  "freshAuthorization": {"version": self.version, "revision": self.metadata["revision"],
                                         "capturedAtUnixSeconds": first["observedAtUnixSeconds"]}}
        self.device_record_path.write_text(json.dumps(record, ensure_ascii=False, indent=2) + NEWLINE, encoding="utf-8")
        print(f"本次授权的设备已记为长期验收设备：{self.device_record_path}")

    def ensure_host_session(self) -> None:
        """The receiver resumes the recorded host task, so a Claude Code session must exist before any wake."""
        if self.host_type != "claude_code":
            return
        task = self.host["taskId"]
        projects = Path.home() / ".claude" / "projects"
        if any(projects.glob(f"*/{task}.jsonl")):
            return
        prompt = (f"这是 Agent Room {self.label} 发布验收专用的会话。之后会有房间消息转交给你，"
                  "请按转交时的要求回复。现在只需回复：已就绪。")
        command = [self.host["executable"], "-p", prompt, "--session-id", task, "--output-format", "json"]
        if self.host.get("model"):
            command += ["--model", self.host["model"]]
        result = subprocess.run(command, cwd=self.host["workspace"], env=self.host_env(), capture_output=True,
                                encoding="utf-8", timeout=300, creationflags=NO_WINDOW, check=False)
        if result.returncode or not any(projects.glob(f"*/{task}.jsonl")):
            raise ReleaseFailure(f"无法在验收工作区创建宿主会话：{(result.stderr or result.stdout).strip()[-300:]}")
        print("已在验收工作区创建宿主会话。")

    def authorized(self) -> bool:
        names = [event.get("event") for event in self.events("bridge")]
        if self.device_mode() == "reused":
            return "ready" in names and "authorization_required" not in names
        return "device_authorized" in names and "ready" in names

    def pending_code(self) -> dict[str, Any] | None:
        events = self.events("bridge")
        if any(event.get("event") == "device_authorized" for event in events):
            return None
        pending = [event for event in events if event.get("event") == "authorization_required"]
        return pending[-1] if pending else None

    def start(self) -> int:
        self.qa.mkdir(exist_ok=True)
        self.ensure_host_session()
        self.apply_device(self.decide_device())
        self.data.mkdir(parents=True, exist_ok=True)
        if self.authorized():
            print("设备已授权，Bridge 就绪；继续运行 run。")
            return 0
        reused = self.device_mode() == "reused"
        started = self.path("bridge-process.json")
        if started.exists() and not reused:
            record = load(started)
            code = self.pending_code()
            fresh = code is not None and time.time() < record["startedAtUnixSeconds"] + int(code.get("expiresInSeconds", 600)) - 30
            if fresh and self.alive("bridge-process.json"):
                return self.announce(record, code)
            # The previous code expired (or the Bridge exited): keep the isolated profile, request a new code.
            self.stop("bridge-process.json")
        before = len(self.events("bridge"))
        record = self.background([self.executable("bridge")], "bridge", self.bridge_env())
        deadline = time.time() + 45
        while time.time() < deadline:
            events = self.events("bridge")[before:]
            if reused and any(event.get("event") == "ready" for event in events):
                print("复用长期验收设备：Bridge 用已保存的授权直接就绪，这一版不需要设备码。继续运行 run。")
                return 0
            if events and events[-1].get("event") == "authorization_required":
                if reused:
                    # The saved authorization no longer works; a fresh profile keeps the evidence honest.
                    self.stop("bridge-process.json")
                    self.save("device-mode.json", {"mode": "fresh", "reason": "长期验收设备的授权已失效。"},
                              overwrite=True)
                    self.use_fresh_device()
                    return self.start()
                return self.announce(record, events[-1])
            time.sleep(1)
        raise ReleaseFailure("Bridge 45 秒内没有申请设备码；查看 bridge.private.stderr。")

    @staticmethod
    def announce(record: dict[str, Any], code: dict[str, Any]) -> int:
        expires = record["startedAtUnixSeconds"] + int(code.get("expiresInSeconds", 600))
        print(json.dumps({
            "approve": code["verificationUri"],
            "expiresAtUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(expires)),
        }, ensure_ascii=False))
        print("请维护者在浏览器中批准设备码；本工具不会代为批准。批准后运行 run。")
        return WAITING

    # ------------------------------------------------------------------ desktop over CDP

    def cdp(self, script: Path | None = None, *, timeout: int = 180) -> Any:
        args = ["node", str(CDP_RUNNER), "--port", str(self.cdp_port), "--url-prefix", self.origin]
        if script is not None:
            args.insert(2, str(script))
        result = subprocess.run(args, capture_output=True, encoding="utf-8", timeout=timeout)
        if result.returncode:
            raise ReleaseFailure((result.stderr.strip() or result.stdout.strip())[-1500:])
        return json.loads(result.stdout.strip().splitlines()[-1])

    def cdp_ready(self) -> bool:
        try:
            return str(self.cdp(timeout=30).get("url", "")).startswith(self.origin)
        except (ReleaseFailure, subprocess.TimeoutExpired, json.JSONDecodeError):
            return False

    def relaunch_desktop(self, *, debugging: bool) -> None:
        script = relaunch_script(self.desktop_executable, self.cdp_port if debugging else None)
        subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True, creationflags=NO_WINDOW)

    def ensure_desktop(self) -> None:
        if self.cdp_ready():
            return
        print("以本机调试端口重启桌面应用，用于驱动真实界面…")
        self.relaunch_desktop(debugging=True)
        deadline = time.time() + 90
        while time.time() < deadline:
            if self.cdp_ready():
                return
            time.sleep(3)
        raise ReleaseFailure("桌面应用 90 秒内没有通过调试端口打开页面；确认已登录并重试。")

    def page(self, name: str, source: str, destination: str | None = None) -> Any:
        script = self.path(name)
        script.write_text(source, encoding="utf-8")
        result = self.cdp(script)
        if destination is not None:
            self.save(destination, result)
        return result

    # ------------------------------------------------------------------ steps

    def join(self) -> None:
        if self.device_mode() == "reused":
            record = load(self.path("device-mode.json"))["record"]
            resumed = self.cli("--profile", record["profileId"], "resume")
            if resumed["identity"]["agent"]["agentId"] != record["agentId"]:
                raise ReleaseFailure("长期验收设备恢复出的人物与记录不一致。")
            self.save("joined.private.json", {"profileId": record["profileId"], "identity": resumed["identity"]})
            print(json.dumps({"profileId": record["profileId"], "agentId": record["agentId"], "reused": True}))
            return
        identifier = self.cli("id")
        identifier = identifier.get("submissionId", identifier.get("id")) if isinstance(identifier, dict) else None
        if not isinstance(identifier, str):
            raise ReleaseFailure("无法生成验收会话标识。")
        invite = {"version": 1, "sessionKey": identifier, "displayName": self.agent_name,
                  "roomId": self.room_id, "catalogId": self.catalog_id}
        encoded = base64.urlsafe_b64encode(json.dumps(invite).encode()).decode().rstrip("=")
        joined = self.cli("join", "--invite", encoded)
        self.save("invitation.json", invite)
        self.save("joined.private.json", joined)
        print(json.dumps({"profileId": joined["profileId"], "agentId": joined["identity"]["agent"]["agentId"]}))

    def joined(self) -> dict[str, Any]:
        return load(self.path("joined.private.json"))

    def invitation(self) -> dict[str, Any]:
        """The invite this profile joined with.

        A reused long-lived device joined in an earlier release and only resumes here, so this release never
        wrote the invite. The CLI's profile id is that invite's session key, and the room comes from the
        resumed identity."""
        path = self.path("invitation.json")
        if path.exists():
            return load(path)
        if self.device_mode() != "reused":
            raise ReleaseFailure("缺少本次加入时的邀请记录。")
        record = load(self.path("device-mode.json"))["record"]
        identity = self.joined()["identity"]
        return {"version": 1, "sessionKey": record["profileId"], "displayName": record["agentName"],
                "roomId": identity["roomId"], "catalogId": identity["roomCatalogId"]}

    def register(self) -> None:
        result = self.cli("--profile", self.joined()["profileId"], "register", "--host",
                          self.host_type.replace("_", "-"), "--task-id", self.host["taskId"],
                          "--workspace", self.host["workspace"])
        self.save("registered.private.json", result)

    def create_grant(self) -> None:
        identity = self.joined()["identity"]
        payload = grant_payload(self.cli("id")["id"], identity)
        self.save("grant-input.json", payload)
        result = self.page("create-grant.js", create_grant_js(self.api, self.version, payload))
        grant = result["grant"]
        if grant["status"] != "active" or grant["messageKinds"] != ["reply"] or grant["agentInstanceId"] != identity["instanceId"]:
            raise ReleaseFailure("验收授权不是只针对这个实例的回复授权。")
        self.save("authorization.private.json", result)

    def bind(self) -> None:
        joined = self.joined()
        invite = self.invitation()
        authorization = load(self.path("authorization.private.json"))
        grant = authorization["grant"]
        if (grant["agentId"], grant["agentInstanceId"], grant["roomCatalogId"]) != (
                joined["identity"]["agent"]["agentId"], joined["identity"]["instanceId"], invite["catalogId"]):
            raise ReleaseFailure("授权与验收身份不一致。")
        self.save("binding.private.json", {
            "session": {"sessionKey": invite["sessionKey"], "displayName": invite["displayName"],
                        "room": {"catalogId": invite["catalogId"], "roomId": invite["roomId"]}},
            "policy": {"roomId": invite["roomId"], "allowedPrincipalId": authorization["principalId"]},
            "automationGrantId": grant["grantId"], "start": {"mode": "now"},
            "host": {"hostType": self.host_type, "taskId": self.host["taskId"], "executable": self.host["executable"],
                     "mcpExecutable": self.executable("mcp-server"), "workspace": self.host["workspace"]},
        })

    def doctor(self) -> None:
        result = self.cli("receiver", "doctor", "--binding", str(self.path("binding.private.json")), timeout=240)
        if not (result.get("attachmentReadable") and result.get("replyContractHonored")):
            raise ReleaseFailure(f"receiver doctor 未通过：{result}")
        self.save("doctor.json", result)

    def prepare(self) -> None:
        source = self.qa / "attachment-source"
        source.mkdir(exist_ok=True)
        canary_path = source / self.canary_name
        if not canary_path.exists():
            # Outside the host workspace: only an Agent Room delivered attachment can reveal it.
            canary_path.write_text(f"{self.slug.upper()}-{secrets.token_hex(6).upper()}\n", encoding="utf-8")
        workspace = Path(self.host["workspace"]).resolve()
        if workspace in canary_path.resolve().parents:
            raise ReleaseFailure("验证码不能放在宿主工作区里。")
        identity = self.joined()["identity"]
        recipient = identity["agent"]["matrixUserId"]
        messages = message_texts(self.label)
        for number in (1, 2):
            attachment = canary_path if number == 2 else None
            self.path(f"send-message-{number}.js").write_text(
                send_message_js(number, messages[number], recipient, self.room_id, attachment), encoding="utf-8")
        target = {"agentId": identity["agent"]["agentId"], "catalogId": identity["roomCatalogId"], "nextDeviceId": None}
        for action in ("read", "takeover"):
            self.path(f"{action}-reception.js").write_text(reception_js(self.api, action, target), encoding="utf-8")
        canary = canary_path.read_text(encoding="utf-8").strip()
        self.path("verify-visible-replies.js").write_text(verify_replies_js(self.agent_name, canary), encoding="utf-8")
        grant_id = load(self.path("authorization.private.json"))["grant"]["grantId"]
        self.path("revoke-grant.js").write_text(revoke_grant_js(self.api, grant_id), encoding="utf-8")

    def receiver_events(self) -> list[dict[str, Any]]:
        path = self.path("receiver.private.jsonl")
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]

    def start_receiver(self) -> None:
        before = len(self.receiver_events())
        args = [self.executable("agent-cli"), "--data-root", str(self.data), "--connection", self.service,
                "receive", "--binding", str(self.path("binding.private.json"))]
        self.background(args, "receiver", self.host_env())
        deadline = time.time() + 90
        while time.time() < deadline:
            fresh = self.receiver_events()[before:]
            if any((r.get("data") or {}).get("type") == "ready" for r in fresh):
                return
            errors = [r.get("error") for r in fresh if r.get("ok") is False]
            if errors:
                raise ReleaseFailure(f"接收端启动失败：{errors[-1]}")
            time.sleep(2)
        raise ReleaseFailure("接收端 90 秒内没有就绪。")

    def open_room(self) -> None:
        target = (f"{self.origin}/lobby/{self.catalog_id}/instance/"
                  f"{urllib.parse.quote(self.room_id, safe='')}?view=conversation")
        self.page("goto-room.js", goto_room_js(target), "room-opened.json")

    def settled_deliveries(self) -> list[dict[str, Any]]:
        return [r["data"]["record"] for r in self.receiver_events()
                if r.get("ok") and (r.get("data") or {}).get("type") == "delivery"
                and r["data"]["record"].get("stage") in ("replied", "failed", "skipped")]

    def send(self, number: int) -> None:
        result = self.page(f"send-message-{number}.js", self.path(f"send-message-{number}.js").read_text(encoding="utf-8"))
        self.save(f"incoming-{number}.json", result)
        deadline = time.time() + (240 if number == 1 else 300)
        while time.time() < deadline:
            settled = self.settled_deliveries()
            if len(settled) >= number:
                if any(item.get("stage") != "replied" for item in settled):
                    raise ReleaseFailure(f"投递没有得到回复：{settled[-1]}")
                return
            time.sleep(3)
        raise ReleaseFailure(f"第 {number} 条消息在时限内没有得到已验证的回复。")

    def verify_replies(self) -> None:
        self.page("verify-visible-replies.js", self.path("verify-visible-replies.js").read_text(encoding="utf-8"),
                  "verified-replies.json")

    def observe(self, label: str) -> dict[str, Any]:
        state = self.cli("receiver", "inspect", "--binding", str(self.path("binding.private.json")))
        raw = self.path("receiver.private.jsonl").read_bytes()
        lines = raw.splitlines(keepends=True)
        records = [json.loads(line) for line in lines if line.endswith(b"\n")]
        observation = {
            "observedAtUnixSeconds": int(time.time()),
            "process": load(self.path("receiver-process.json")),
            "receiverProcessAlive": self.alive("receiver-process.json"),
            "events": [r["data"] for r in records if r.get("ok") is True],
            "errors": [r["error"] for r in records if r.get("ok") is False],
            "logSha256": hashlib.sha256(raw).hexdigest(),
            "partialLastLine": bool(lines and not lines[-1].endswith(b"\n")),
            "state": {key: state.get(key) for key in ("enabled", "execution", "checkpoint", "lastDelivery")},
        }
        self.save(f"observation-{label}.json", observation)
        return observation

    def presence(self, label: str) -> None:
        agent_id = self.joined()["identity"]["agent"]["agentId"]
        cursor, seen = None, set()
        for _ in range(200):
            paging = [] if cursor is None else ["--after", cursor]
            result = self.cli("--profile", self.joined()["profileId"], "presence", "--include-archived", *paging)
            matches = [entry for entry in result["entries"] if entry["agent"]["agentId"] == agent_id]
            if matches:
                self.save(f"presence-{label}.json", {"observedAtUnixSeconds": int(time.time()), "entry": matches[0]})
                return
            cursor = result.get("nextCursor")
            if cursor is None or cursor in seen:
                raise ReleaseFailure("验收人物不在在场名单里。")
            seen.add(cursor)
        raise ReleaseFailure("在场名单超出分页上限。")

    def idle(self) -> None:
        before = self.path("observation-idle-before.json")
        started = load(before)["observedAtUnixSeconds"] if before.exists() else self.observe("idle-before")["observedAtUnixSeconds"]
        if not self.path("presence-during-idle.json").exists():
            time.sleep(max(0, started + 30 - time.time()))
            self.presence("during-idle")
        time.sleep(max(0, started + IDLE_OBSERVATION_SECONDS - time.time()))
        self.observe("idle-after")

    def wait_receiver_exit(self, seconds: int = 90) -> None:
        deadline = time.time() + seconds
        while self.alive("receiver-process.json"):
            if time.time() > deadline:
                raise ReleaseFailure("接管后接收端进程没有退出。")
            time.sleep(2)

    def takeover(self) -> None:
        if not self.path("reception-takeover.json").exists():
            self.page("takeover-reception.js", self.path("takeover-reception.js").read_text(encoding="utf-8"),
                      "reception-takeover.json")
        self.wait_receiver_exit()
        if not self.path("observation-after-takeover.json").exists():
            self.observe("after-takeover")
        self.page("read-reception.js", self.path("read-reception.js").read_text(encoding="utf-8"), "reception-stopped.json")

    def resume(self) -> None:
        if not self.path("observation-after-resume.json").exists():
            self.start_receiver()
            time.sleep(15)
            self.observe("after-resume")
        self.page("read-reception.js", self.path("read-reception.js").read_text(encoding="utf-8"), "reception-resumed.json")

    def assemble(self) -> None:
        assemble_reports(self)
        self.remember_device()

    # ------------------------------------------------------------------ upgrade

    def installed(self) -> tuple[list[str], str, dict[str, Any]]:
        """The long-lived upgrade identity joined through the installed CLI on the owner's own Bridge."""
        record = load(Path(self.config["upgrade"]["identity"]))
        configuration = record["configuration"]
        return [configuration["command"], *configuration["args"]], record["joined"]["data"]["profileId"], record

    def installed_cli(self, *args: str, timeout: int = 60) -> Any:
        base, profile, _ = self.installed()
        result = subprocess.run([*base, "--profile", profile, *args], capture_output=True, encoding="utf-8",
                                timeout=timeout, creationflags=NO_WINDOW)
        envelope = json.loads(result.stdout)
        if result.returncode or envelope.get("ok") is not True:
            raise ReleaseFailure(f"已安装 CLI 的 {args[0]} 失败：{envelope.get('error')}")
        return envelope["data"]

    def write_record(self, name: str, value: dict[str, Any]) -> None:
        with (self.work / name).open("x", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2)
            stream.write(NEWLINE)

    def upgrade_baseline(self) -> None:
        if self.migration:
            self.migration_baseline()
            return
        base, profile, record = self.installed()
        version = subprocess.check_output([base[0], "--version"], encoding="utf-8", timeout=15).strip()
        version = version.removeprefix("agent-room ")
        if version == self.version:
            raise ReleaseFailure("本机已经是候选版本；升级验收要先装着上一个公开版本。")
        identity = self.installed_cli("resume")["identity"]
        if identity["agent"]["agentId"] != record["joined"]["data"]["identity"]["agent"]["agentId"]:
            raise ReleaseFailure("已安装版本恢复出的人物不是升级验收身份。")
        inbox = self.installed_cli("read", "--wait", "0")
        messages = inbox.get("messages", inbox.get("previews"))
        events = [message["eventId"] for message in messages]
        if not events:
            raise ReleaseFailure("升级前需要至少一条真实的未确认投递。")
        self.write_record("upgrade-baseline.json", {
            "version": version, "capturedAtUnixSeconds": int(time.time()), "profileId": profile,
            "agentId": identity["agent"]["agentId"], "roomId": identity["roomId"],
            "pendingEventIds": events, "acknowledged": False})

    def bridge_data_dir(self) -> Path:
        configured = self.config["upgrade"].get("bridgeDataDir")
        return Path(configured) if configured else Path(os.environ["LOCALAPPDATA"]) / "AgentRoom" / "Bridge"

    def migration_baseline(self) -> None:
        """A server move carries no identity or deliveries across: record the installed version and the old
        server's default agent target, which the upgraded desktop must retire instead of reusing."""
        desktop = Path(self.config["upgrade"]["installDir"]) / "agent-room-desktop.exe"
        version = subprocess.check_output([str(desktop), "--installer-version"], encoding="utf-8", timeout=15).strip()
        if version == self.version:
            raise ReleaseFailure("本机已经是候选版本；升级验收要先装着上一个公开版本。")
        target = self.bridge_data_dir() / "desktop" / "agent-target.json"
        self.write_record("upgrade-baseline.json", {
            "version": version, "capturedAtUnixSeconds": int(time.time()), "serverMigration": self.migration,
            "agentTargetSha256": digest(target) if target.exists() else None})

    def previous_state_retired(self) -> dict[str, Any]:
        """The upgraded desktop recorded the new server and moved the old default agent target aside."""
        data = self.bridge_data_dir()
        record = data / "deployment.json"
        if not record.exists() or load(record).get("controlPlaneUrl") != self.config["controlPlaneUrl"]:
            raise ReleaseFailure("升级后的桌面没有把本机状态记到新服务器名下。")
        old = load(self.work / "upgrade-baseline.json")["agentTargetSha256"]
        if old is not None:
            if not any(digest(path) == old for path in data.glob("retired/*/desktop/agent-target.json")):
                raise ReleaseFailure("旧服务器的默认 Agent 目标没有被移进 retired。")
            active = data / "desktop" / "agent-target.json"
            if active.exists() and digest(active) == old:
                raise ReleaseFailure("升级后仍在使用旧服务器的默认 Agent 目标。")
        return {"deploymentRecorded": True, "previousAgentTargetRetired": old is not None}

    def upgrade_install(self) -> None:
        install_dir = Path(self.config["upgrade"]["installDir"])
        asset = windows_installer(self.manifest)
        installer =(self.candidate / asset["url"].rsplit("/", 1)[1]).resolve(strict=True)
        if (installer.parent != self.candidate.resolve() or digest(installer) != asset["sha256"]
                or installer.stat().st_size != asset["byteLength"]):
            raise ReleaseFailure("安装器不是清单中的签名产物。")
        desktop = install_dir / "agent-room-desktop.exe"

        def desktop_version() -> str:
            return subprocess.check_output([str(desktop), "--installer-version"], encoding="utf-8", timeout=15).strip()

        previous = desktop_version()
        if previous != load(self.work / "upgrade-baseline.json")["version"]:
            raise ReleaseFailure("已安装的旧版本与升级前记录不一致。")
        completed = subprocess.run([str(installer), "/S", "/NS"], timeout=300, creationflags=NO_WINDOW, check=False)
        if completed.returncode != 0:
            raise ReleaseFailure(f"安装器退出码 {completed.returncode}。")
        cli_version = subprocess.check_output([str(install_dir / "agent-room.exe"), "--version"],
                                              encoding="utf-8", timeout=15).strip()
        if desktop_version() != self.version or cli_version != f"agent-room {self.version}":
            raise ReleaseFailure("安装后的桌面或 CLI 不是候选版本。")
        hashes = {}
        for name, filename in {"bridge": "agent-room-bridge.exe", "agent-cli": "agent-room.exe",
                               "mcp-server": "agent-room-mcp.exe"}.items():
            artifact, = [a for a in self.manifest["artifacts"] if a["name"] == name and a["platform"] == "windows-x86_64"]
            path = install_dir / filename
            if digest(path) != artifact["sha256"] or path.stat().st_size != artifact["byteLength"]:
                raise ReleaseFailure(f"安装后的 {filename} 与候选不一致。")
            hashes[name] = artifact["sha256"]
        self.write_record("installed-verification.json", {
            "version": self.version, "revision": self.metadata["revision"], "verifiedAtUnixSeconds": int(time.time()),
            "previousVersion": previous, "installerExitCode": 0, "installerSha256": asset["sha256"],
            "desktopVersion": self.version, "cliVersion": self.version, "runtimeHashesMatched": True,
            "runtimeSha256": hashes})

    def upgrade_session(self) -> None:
        self.qa.mkdir(exist_ok=True)
        self.ensure_desktop()
        if self.migration:
            self.migrated_session()
            return
        result = self.page("capture-native-session.js", native_session_js(self.origin, self.api, self.version))
        if not (result.get("loginRestored") and result.get("bridgeReady")):
            raise ReleaseFailure(f"升级后登录或 Bridge 没有恢复：{result}")
        self.write_record("native-session-restoration.json", result)

    def migrated_session(self) -> None:
        """The upgraded app must hold no login on the new server; then the owner signs in there anew."""
        initial = self.work / "migrated-session-initial.json"
        script = migrated_session_js(self.origin, self.api, self.version)
        if not initial.exists():
            observed = self.page("observe-migrated-session.js", script)
            if observed["signedIn"]:
                raise ReleaseFailure("升级后应用已经登录了新服务器，无法证明旧登录没有被沿用。")
            self.write_record(initial.name, {**observed, **self.previous_state_retired()})
        print(f"请在桌面应用里登录新服务器 {self.migration['to']}（新服务器上要重新注册账号）；"
              "登录且本机 Agent 就绪后自动继续。", flush=True)
        deadline = time.time() + MIGRATION_SIGN_IN_SECONDS
        while True:
            observed = self.page("observe-migrated-session.js", script)
            if observed["signedIn"] and observed["bridgeReady"]:
                break
            if time.time() > deadline:
                raise ReleaseFailure(f"{MIGRATION_SIGN_IN_SECONDS // 60} 分钟内没有在新服务器上登录并就绪："
                                     f"{observed}；登录后重跑 upgrade 继续。")
            time.sleep(10)
        self.write_record("native-session-restoration.json", {
            **observed, "previousLoginNotReused": True, "signedInToNewServer": True,
            "previousAgentTargetRetired": load(initial)["previousAgentTargetRetired"],
            "initialObservationSha256": digest(initial)})

    def upgrade_verify(self) -> None:
        baseline = load(self.work / "upgrade-baseline.json")
        installed = load(self.work / "installed-verification.json")
        native = load(self.work / "native-session-restoration.json")
        if not (baseline["version"] == installed["previousVersion"]
                and installed["version"] == native["currentVersion"] == self.version
                and native["observedAtUnixSeconds"] >= installed["verifiedAtUnixSeconds"]):
            raise ReleaseFailure("升级记录彼此不一致。")
        if self.migration:
            if not (native.get("previousLoginNotReused") and native.get("signedInToNewServer")
                    and native.get("bridgeReady")):
                raise ReleaseFailure("迁移后的登录记录不完整。")
            self.write_record("usability-evidence-upgrade.json", {
                "previousVersion": baseline["version"], "currentVersion": self.version,
                "observedAtUnixSeconds": int(time.time()), "installerSha256": installed["installerSha256"],
                "runtimeHashesMatched": True, "upgradeMode": "server-migration", "serverMigration": self.migration,
                "previousLoginNotReused": True, "previousAgentTargetRetired": native["previousAgentTargetRetired"],
                "signedInToNewServer": True, "bridgeReady": True})
            return
        identity = self.installed_cli("resume")["identity"]
        try:
            if identity["agent"]["agentId"] != baseline["agentId"] or identity["roomId"] != baseline["roomId"]:
                raise ReleaseFailure("升级后人物或房间变了。")
            if self.installed_cli("whoami")["summary"]["connectionState"] != "ready":
                raise ReleaseFailure("升级后连接没有就绪。")
            inbox = self.installed_cli("read", "--wait", "0", "--limit", "50")
            actual = {item["eventId"] for item in inbox["previews"]}
            if not set(baseline["pendingEventIds"]) <= actual:
                raise ReleaseFailure("升级后丢失了未确认投递。")
        finally:
            self.installed_cli("leave")
        self.write_record("usability-evidence-upgrade.json", {
            "previousVersion": baseline["version"], "currentVersion": self.version,
            "observedAtUnixSeconds": int(time.time()), "installerSha256": installed["installerSha256"],
            "runtimeHashesMatched": True, "loginRestored": True, "identityPreserved": True, "roomPreserved": True,
            "pendingDeliveryPreserved": True, "previousPendingCount": len(baseline["pendingEventIds"]),
            "currentPendingCount": len(actual), "acknowledgedDuringVerification": False})

    def baseline(self) -> int:
        """Record the installed previous version before the server upgrade, so the old-client checks
        on both sides of it and the later desktop upgrade compare against the same pending deliveries."""
        if (self.work / "upgrade-baseline.json").exists():
            print("升级前基线已存在，沿用。")
            return 0
        self.upgrade_baseline()
        print("升级前基线已记录；服务端升级和旧客户端检查之后运行 upgrade。")
        return 0

    def upgrade(self) -> int:
        for name, step, record in [
            ("baseline", self.upgrade_baseline, self.work / "upgrade-baseline.json"),
            ("install", self.upgrade_install, self.work / "installed-verification.json"),
            ("session", self.upgrade_session, self.work / "native-session-restoration.json"),
            ("verify", self.upgrade_verify, self.work / "usability-evidence-upgrade.json"),
        ]:
            if not record.exists():
                print(f"→ upgrade {name}", flush=True)
                step()
        print("升级验收记录已就绪；接下来运行 start。")
        return 0

    # ------------------------------------------------------------------ orchestration

    def steps(self) -> list[tuple[str, Callable[[], None], Path]]:
        return [
            ("join", self.join, self.path("joined.private.json")),
            ("register", self.register, self.path("registered.private.json")),
            ("grant", self.create_grant, self.path("authorization.private.json")),
            ("bind", self.bind, self.path("binding.private.json")),
            ("doctor", self.doctor, self.path("doctor.json")),
            ("prepare", self.prepare, self.path("revoke-grant.js")),
            ("receive", self.start_receiver, self.path("receiver-process.json")),
            ("room", self.open_room, self.path("room-opened.json")),
            ("message-1", lambda: self.send(1), self.path("incoming-1.json")),
            ("message-2", lambda: self.send(2), self.path("incoming-2.json")),
            ("replies", self.verify_replies, self.path("verified-replies.json")),
            ("idle", self.idle, self.path("observation-idle-after.json")),
            ("takeover", self.takeover, self.path("reception-stopped.json")),
            ("resume", self.resume, self.path("reception-resumed.json")),
            ("assemble", self.assemble, self.work / "release-usability-acceptance.json"),
        ]

    def run(self) -> int:
        if not self.authorized():
            print("设备码尚未获批。先运行 start，并请维护者批准。")
            return WAITING
        needs_desktop = {"grant", "room", "message-1", "message-2", "replies", "takeover", "resume"}
        for name, step, record in self.steps():
            if record.exists():
                continue
            if name in needs_desktop:
                self.ensure_desktop()
            print(f"→ {name}", flush=True)
            step()
        self.cleanup()
        print("三份验收已汇总到候选目录；接下来运行 release_flow 或公开发行步骤。")
        return 0

    def cleanup(self) -> None:
        if self.alive("receiver-process.json"):
            self.ensure_desktop()
            if not self.path("cleanup-takeover.json").exists():
                self.page("takeover-reception.js", self.path("takeover-reception.js").read_text(encoding="utf-8"),
                          "cleanup-takeover.json")
            self.wait_receiver_exit()
        if self.path("authorization.private.json").exists() and not self.path("grant-revoked.json").exists():
            self.ensure_desktop()
            self.page("revoke-grant.js", self.path("revoke-grant.js").read_text(encoding="utf-8"), "grant-revoked.json")
        if self.path("joined.private.json").exists() and not self.path("left-room.json").exists():
            if not self.alive("bridge-process.json") or not self.authorized():
                raise ReleaseFailure("隔离 Bridge 已停止，无法退出房间；手动启动后重试 cleanup。")
            result = self.cli("--profile", self.joined()["profileId"], "leave")
            self.save("left-room.json", {"left": True, "observedAtUnixSeconds": int(time.time()), "result": result})
        self.stop("bridge-process.json")
        if self.cdp_ready():
            self.relaunch_desktop(debugging=False)
            if self.cdp_ready():
                raise ReleaseFailure(f"桌面应用重启后调试端口 {self.cdp_port} 仍然开着；退出桌面应用后重试 cleanup。")
        print("已清理：授权撤销、人物退出房间、隔离 Bridge 与接收端停止、桌面应用恢复普通启动。")

    def status(self) -> int:
        done = [name for name, _, record in self.steps() if record.exists()]
        code = self.pending_code()
        print(json.dumps({"version": self.version, "deviceMode": self.device_mode(), "authorized": self.authorized(),
                          "pendingDeviceCode": code is not None, "completed": done,
                          "cleanedUp": self.path("left-room.json").exists()}, ensure_ascii=False))
        return 0


# ---------------------------------------------------------------------- local processes


def windows_installer(manifest: dict[str, Any]) -> dict[str, Any]:
    """Pick the installer this Windows workstation upgrades with.

    Since Alpha 45 the manifest also carries the macOS disk image, so the installer must be chosen by
    platform like every other binary here, not by being the only one."""
    installers = [a for a in manifest["artifacts"]
                  if a["kind"] == "installer" and a["platform"] == "windows-x86_64"]
    if len(installers) != 1:
        raise ReleaseFailure("候选清单里没有唯一的 Windows 安装器。")
    return installers[0]


def ps_literal(value: str) -> str:
    """A single-quoted PowerShell string literal."""
    return "'" + value.replace("'", "''") + "'"


def liveness_script(pid: int, executable: str) -> str:
    """Print yes while the pid still runs the recorded executable.

    Get-Process leaves $? false for a pid that already exited, even with SilentlyContinue, and
    powershell -Command then exits 1; the explicit exit 0 keeps "not running" an answer, not an error.
    """
    return (f"$p = Get-Process -Id {int(pid)} -ErrorAction SilentlyContinue; "
            "if ($p -and $p.Path -and [IO.Path]::GetFullPath($p.Path) -eq "
            f"[IO.Path]::GetFullPath({ps_literal(executable)})) {{ 'yes' }}; exit 0")


def relaunch_script(executable: str, cdp_port: int | None) -> str:
    """Close the installed desktop app, then start it with or without the WebView debugging port.

    Paths are compared after GetFullPath because the configuration may spell them with forward slashes
    while Get-Process reports backslashes. A missed match leaves the old instance running, and the new
    launch only hands over to it through the single-instance guard, so the port state never changes.
    """
    return (
        f"$app = [IO.Path]::GetFullPath({ps_literal(ntpath.normpath(executable))}); "
        "foreach ($p in @(Get-Process -Name 'agent-room-desktop' -ErrorAction SilentlyContinue | "
        "Where-Object { $_.Path -and [IO.Path]::GetFullPath($_.Path) -eq $app })) { $null = $p.CloseMainWindow(); "
        "if (-not $p.WaitForExit(5000)) { Stop-Process -Id $p.Id; $p.WaitForExit(5000) | Out-Null } }; "
        # The WebView browser process can outlive the app for a moment; let it exit so the new instance
        # starts its own with the requested arguments.
        "$deadline = (Get-Date).AddSeconds(20); "
        "while ((Get-Date) -lt $deadline -and @(Get-CimInstance Win32_Process -Filter 'Name = ''msedgewebview2.exe''' | "
        "Where-Object { $_.CommandLine -like '*--webview-exe-name=agent-room-desktop.exe*' }).Count) "
        "{ Start-Sleep -Milliseconds 500 }; "
        + (f"$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port={int(cdp_port)}'; "
           if cdp_port is not None else
           "Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue; ")
        + "Start-Process -FilePath $app | Out-Null"
    )


# ---------------------------------------------------------------------- page functions


def grant_payload(grant_id: str, identity: dict[str, Any]) -> dict[str, Any]:
    """One hour, five replies, only this QA instance in only this room."""
    return {"grantId": grant_id, "input": {
        "agentId": identity["agent"]["agentId"], "agentInstanceId": identity["instanceId"],
        "roomCatalogId": identity["roomCatalogId"], "audience": "any_room_member",
        "messageKinds": ["reply"], "impactAcknowledged": True, "lifetimeSeconds": 3600,
        "maxMessagesPerMinute": 10, "maxTotalMessages": 5, "requiresRiskScan": True}}


def fill(template: str, values: dict[str, str]) -> str:
    """Replace @@TOKEN@@ placeholders; tokens cannot collide with each other or with inserted text."""
    for key, value in values.items():
        token = f"@@{key}@@"
        if template.count(token) < 1:
            raise ValueError(f"template has no {token}")
        template = template.replace(token, value)
    if re.search(r"@@[A-Z_]+@@", template):
        raise ValueError("template still has an unfilled placeholder")
    return template


def create_grant_js(api: str, version: str, payload: dict[str, Any]) -> str:
    return fill("""async page => {
  return await page.evaluate(async payload => {
    const base = @@API@@;
    const readiness = await fetch(base + "/health/ready");
    const health = await readiness.json();
    if (!readiness.ok || health.version !== @@VERSION@@) throw new Error("Candidate server is not running");
    const response = await fetch(base + "/auth/session", {credentials: "include"});
    const session = await response.json();
    if (!response.ok || typeof session.principalId !== "string") throw new Error("Owner session not restored");
    const created = await fetch(base + "/automation-grants", {
      method: "POST", credentials: "include", headers: {
        "Content-Type": "application/json", "Idempotency-Key": payload.grantId
      }, body: JSON.stringify(payload.input)
    });
    const grant = await created.json();
    if (!created.ok) throw new Error("QA grant failed: " + JSON.stringify(grant));
    return {principalId: session.principalId, grant, observedAtUnixSeconds: Math.floor(Date.now() / 1000)};
  }, @@PAYLOAD@@);
}""", {"API": json.dumps(api), "VERSION": json.dumps(version), "PAYLOAD": json.dumps(payload)})


def message_texts(label: str) -> dict[int, str]:
    return {
        1: f"这是 {label} 真实接待验收的第一条消息，请简短回复：第一条消息已收到。",
        2: f"这是 {label} 真实接待验收的第二条消息，请读取本条消息的附件，并回复附件里的验证码。",
    }


def send_message_js(number: int, text: str, recipient: str, room: str, attachment: Path | None) -> str:
    attach = "" if attachment is None else (
        "await page.getByLabel(" + js_name("attach") + ").setInputFiles(" + json.dumps(str(attachment)) + ");" + chr(10)
        + "      await page.getByText(" + json.dumps(attachment.name) + ", {exact: true}).waitFor();")
    return fill("""async page => {
      const input = page.getByRole('textbox', {name: @@INPUT@@});
      if (await input.inputValue() !== '') throw new Error('Preserve the existing draft');
      await page.getByRole('combobox', {name: @@MENTION@@}).selectOption(@@RECIPIENT@@);
      await input.fill(@@MESSAGE@@);
      @@ATTACH_STEP@@
      const responsePromise = page.waitForResponse(response => {
        if (response.request().method() !== 'PUT' || !response.url().endsWith('/event-binding')) return false;
        const body = response.request().postDataJSON();
        return body.matrixRoomId === @@ROOM@@;
      }, {timeout: 45000});
      await page.getByRole('button', {name: @@SEND@@}).click();
      const response = await responsePromise;
      const receipt = await response.json();
      if (!response.ok() || receipt.matrixRoomId !== @@ROOM@@ || !receipt.matrixEventId.startsWith('$')) {
        throw new Error('Message publication was not confirmed: ' + response.status());
      }
      await input.waitFor({state: 'visible'});
      return {scenarioMessage: @@NUMBER@@, eventId: receipt.matrixEventId, contentId: receipt.contentId,
        attachmentSent: @@HAS_ATTACHMENT@@, observedAtUnixSeconds: Math.floor(Date.now() / 1000)};
    }""", {
        "INPUT": js_name("input"), "MENTION": js_name("mention"), "SEND": js_name("send"),
        "RECIPIENT": json.dumps(recipient), "MESSAGE": json.dumps(text), "ROOM": json.dumps(room),
        "NUMBER": str(number), "HAS_ATTACHMENT": "true" if attachment is not None else "false",
        "ATTACH_STEP": attach,
    })


def reception_js(api: str, action: str, target: dict[str, Any]) -> str:
    endpoint = "/receptions" if action == "read" else "/receptions/transfer"
    init = "" if action == "read" else "method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(input)"
    pick = ("body.receptions.find(item => item.agentId === input.agentId && item.catalogId === input.catalogId)"
            if action == "read" else "body")
    return fill("""async page => {
      return await page.evaluate(async input => {
        const response = await fetch(@@BASE@@ + @@ENDPOINT@@, {credentials: 'include', cache: 'no-store',
          signal: AbortSignal.timeout(15000), @@INIT@@});
        const body = await response.json();
        if (!response.ok) throw new Error('Reception action failed: ' + response.status);
        const record = @@PICK@@;
        if (!record || record.agentId !== input.agentId || record.catalogId !== input.catalogId) {
          throw new Error('The QA reception was not returned');
        }
        return {record, observedAtUnixSeconds: Math.floor(Date.now() / 1000)};
      }, @@PAYLOAD@@);
    }""", {"BASE": json.dumps(api), "ENDPOINT": json.dumps(endpoint), "INIT": init, "PICK": pick,
           "PAYLOAD": json.dumps(target)})


def verify_replies_js(agent_name: str, canary: str) -> str:
    return fill("""async page => {
  const replies = page.getByRole('log', {name: @@LOG@@})
    .locator('article[data-actor-kind="agent"]').filter({hasText: @@AGENT_NAME@@});
  await replies.filter({hasText: '第一条消息已收到'}).waitFor({state: 'visible', timeout: 30000});
  await replies.filter({hasText: @@CANARY@@}).waitFor({state: 'visible', timeout: 30000});
  const visibleAgentReplies = await replies.count();
  if (visibleAgentReplies !== 2) throw new Error('Expected exactly two visible replies from the QA Agent');
  return {firstReplyTextConfirmed: true, attachmentCanaryConfirmed: true,
    visibleAgentReplies, observedAtUnixSeconds: Math.floor(Date.now()/1000)};
}""", {"LOG": js_name("log"), "AGENT_NAME": json.dumps(agent_name), "CANARY": json.dumps(canary)})


def revoke_grant_js(api: str, grant_id: str) -> str:
    return fill("""async page => {
  return await page.evaluate(async id => {
    const response = await fetch(@@BASE@@ + '/automation-grants/' + id,
      {method: 'DELETE', credentials: 'include', signal: AbortSignal.timeout(15000)});
    if (!response.ok) throw new Error('Test grant cleanup failed: ' + response.status);
    return {revoked: true, observedAtUnixSeconds: Math.floor(Date.now()/1000)};
  }, @@GRANT_ID@@);
}""", {"BASE": json.dumps(api), "GRANT_ID": json.dumps(grant_id)})


def native_session_js(origin: str, api: str, version: str) -> str:
    return fill("""async page => {
  return await page.evaluate(async () => {
    if (location.origin !== @@ORIGIN@@) throw new Error('Expected the installed desktop application');
    const runtime = await window.__TAURI_INTERNALS__.invoke('desktop_runtime_snapshot');
    if (runtime.currentVersion !== @@VERSION@@) throw new Error('The installed candidate is not running');
    const response = await fetch(@@API@@ + '/auth/session', {
      credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(15000),
    });
    const session = await response.json();
    const loginRestored = response.status === 200 && typeof session.principalId === 'string';
    const bridgePhase = runtime.bridge.lifecycle.phase;
    const bridgeReady = ['authorized', 'ready'].includes(bridgePhase) && runtime.bridge.authorization === null;
    if (!loginRestored || !bridgeReady) throw new Error(JSON.stringify({loginRestored, bridgePhase}));
    return {currentVersion: runtime.currentVersion, loginRestored, bridgeReady, bridgePhase,
      httpSessionStatus: response.status, updatesConfigured: runtime.updatesConfigured,
      observedAtUnixSeconds: Math.floor(Date.now() / 1000)};
  });
}""", {"ORIGIN": json.dumps(origin), "API": json.dumps(api), "VERSION": json.dumps(version)})


def migrated_session_js(origin: str, api: str, version: str) -> str:
    """Observe, without failing, whether the upgraded app is signed in to the new server and its Bridge is ready."""
    return fill("""async page => {
  return await page.evaluate(async () => {
    if (location.origin !== @@ORIGIN@@) throw new Error('Expected the installed desktop application');
    const runtime = await window.__TAURI_INTERNALS__.invoke('desktop_runtime_snapshot');
    if (runtime.currentVersion !== @@VERSION@@) throw new Error('The installed candidate is not running');
    const response = await fetch(@@API@@ + '/auth/session', {
      credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(15000),
    });
    const session = response.status === 200 ? await response.json() : null;
    const signedIn = session !== null && typeof session.principalId === 'string';
    const bridgePhase = runtime.bridge.lifecycle.phase;
    const bridgeReady = ['authorized', 'ready'].includes(bridgePhase) && runtime.bridge.authorization === null;
    return {currentVersion: runtime.currentVersion, signedIn, httpSessionStatus: response.status, bridgePhase,
      bridgeReady, updatesConfigured: runtime.updatesConfigured, observedAtUnixSeconds: Math.floor(Date.now() / 1000)};
  });
}""", {"ORIGIN": json.dumps(origin), "API": json.dumps(api), "VERSION": json.dumps(version)})


def goto_room_js(target: str) -> str:
    return fill("async page => { await page.goto(@@TARGET@@, {waitUntil: 'domcontentloaded', timeout: 30000}); "
                "await page.getByRole('textbox', {name: @@INPUT@@}).waitFor({state: 'visible', timeout: 45000}); "
                "return {url: page.url(), observedAtUnixSeconds: Math.floor(Date.now()/1000)}; }",
                {"TARGET": json.dumps(target), "INPUT": js_name("input")})


# ---------------------------------------------------------------------- reports


def deliveries(observed: dict[str, Any], stage: str) -> list[dict[str, Any]]:
    return [event["record"] for event in observed["events"]
            if event["type"] == "delivery" and event["record"]["stage"] == stage]


def assemble_reports(run: Acceptance) -> None:
    """Derive the three reports only from what was observed, then assemble them into the candidate."""
    qa, work, metadata = run.qa, run.work, run.metadata
    lower = metadata["publishedAtUnixSeconds"]
    ci = load(work / "ci-verification.json")
    if not ci.get("allRequiredJobsPassed") or ci.get("revision") != metadata["revision"]:
        raise ReleaseFailure("完整 CI 未在候选提交上通过。")

    def check(condition: bool, message: str) -> None:
        if not condition:
            raise ReleaseFailure(message)

    def observation(label: str) -> dict[str, Any]:
        value = load(qa / f"observation-{label}.json")
        check(value["errors"] == [] and not value["partialLastLine"], f"{label} 观察含错误或截断日志。")
        check(lower <= value["observedAtUnixSeconds"], f"{label} 观察早于候选生成。")
        return value

    def report(scenario: str, checks: dict[str, bool], evidence: list[Path], captured: int, **extra: Any) -> Path:
        check(bool(checks) and all(value is True for value in checks.values()), f"{scenario} 存在未通过的检查。")
        check(lower <= captured <= int(time.time()), f"{scenario} 的采集时间不在候选验收期内。")
        path = work / f"release-usability-{scenario}.json"
        with path.open("x", encoding="utf-8") as stream:
            json.dump({"schemaVersion": 1, "scenario": scenario, "version": metadata["version"],
                       "revision": metadata["revision"], "signedManifestSha256": run.verified["signedManifestSha256"],
                       "capturedAtUnixSeconds": captured, "result": "passed", "fixture": False, "checks": checks,
                       "evidence": [{"path": p.name, "sha256": digest(p), "redacted": True} for p in evidence],
                       **extra}, stream, indent=2)
            stream.write("\n")
        return path

    def evidence(name: str, value: dict[str, Any]) -> Path:
        path = work / name
        with path.open("x", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
        return path

    before, after = observation("idle-before"), observation("idle-after")
    stopped, resumed = observation("after-takeover"), observation("after-resume")
    replies = deliveries(after, "replied")
    incoming = [load(qa / f"incoming-{number}.json") for number in (1, 2)]
    check(len(replies) == len(deliveries(after, "running")) == 2, "空闲结束时应恰好有两次宿主调用和两条回复。")
    check([item["eventId"] for item in replies] == [item["eventId"] for item in incoming], "回复与发送的消息不对应。")
    check(all(item["failure"] is None for item in replies), "回复记录含失败。")
    receipts = [{"eventId": item["replyEventId"], "submissionId": item["submissionId"]} for item in replies]
    check(all(item["eventId"].startswith("$") and UUID7.fullmatch(item["submissionId"]) for item in receipts),
          "回复回执格式无效。")
    check(len({i["eventId"] for i in receipts}) == len({i["submissionId"] for i in receipts}) == 2, "两条回执必须不同。")
    visible = load(qa / "verified-replies.json")
    check(visible["visibleAgentReplies"] == 2 and visible["firstReplyTextConfirmed"]
          and visible["attachmentCanaryConfirmed"], "界面没有显示两条正确回复。")
    check(bool(incoming[1]["attachmentSent"]), "第二条消息必须带附件。")
    idle_seconds = after["observedAtUnixSeconds"] - before["observedAtUnixSeconds"]
    check(idle_seconds >= 60, "空闲观察不足 60 秒。")
    check(len(deliveries(before, "running")) == 2, "空闲期间出现了额外的宿主调用。")
    check(before["receiverProcessAlive"] and after["receiverProcessAlive"], "空闲期间接收端退出。")
    check(before["state"]["checkpoint"] == after["state"]["checkpoint"], "空闲期间游标变化。")
    check(not stopped["receiverProcessAlive"] and stopped["state"]["execution"] is None
          and stopped["state"]["enabled"] is False and stopped["events"][-1]["type"] == "stopped",
          "人工接管没有停止后台接收。")
    server_stopped = load(qa / "reception-stopped.json")["record"]
    server_resumed = load(qa / "reception-resumed.json")["record"]
    check(server_stopped["status"] == "idle", "接管后服务端没有转为 idle。")
    check(resumed["receiverProcessAlive"] and resumed["state"]["execution"] is not None, "交回后接收端没有运行。")
    check(resumed["state"]["checkpoint"] == stopped["state"]["checkpoint"] == after["state"]["checkpoint"],
          "交回后游标变化。")
    check(len(deliveries(resumed, "running")) == len(deliveries(resumed, "replied")) == 2, "交回后重复处理了消息。")
    check(server_resumed["status"] == "active" and server_resumed["progress"] == server_stopped["progress"]
          and server_resumed["runId"] != server_stopped["runId"], "交回后服务端运行或进度不符。")
    presence = load(qa / "presence-during-idle.json")
    check(presence["entry"]["lifecycle"]["reception"] == "waiting"
          and presence["entry"]["lifecycle"]["connection"] == "online", "空闲期间人物不是在线等消息。")

    host = run.host
    reception_evidence = evidence("usability-evidence-continuous-reception.json", {
        "version": metadata["version"], "observedAtUnixSeconds": resumed["observedAtUnixSeconds"],
        "host": HOST_NAMES[run.host_type], "hostType": run.host_type,
        "hostExecutableSha256": digest(Path(host["executable"])),
        "hostTaskIdSha256": hashlib.sha256(host["taskId"].encode()).hexdigest(),
        "hostInvocationCount": 2, "verifiedReplyCount": 2, "incomingEventIds": [i["eventId"] for i in incoming],
        "replyReceipts": receipts, "attachmentCanaryConfirmed": True, "idleDurationSeconds": idle_seconds,
        "hostInvocationCountBeforeIdle": 2, "hostInvocationCountAfterIdle": 2, "idleReceptionState": "waiting",
        "manualTakeoverStoppedProcess": True, "serverStatusAfterTakeover": "idle",
        "serverStatusAfterResume": "active", "resumeKeptCursor": True,
        "cursorBeforeTakeover": after["state"]["checkpoint"], "cursorAfterResume": resumed["state"]["checkpoint"],
        "observations": [{"label": label, "sha256": digest(qa / f"observation-{label}.json")}
                         for label in ["idle-before", "idle-after", "after-takeover", "after-resume"]],
    })
    continuous = report("continuous-reception", {
        "realHostInvoked": True, "twoIncomingMessages": True, "twoRepliesVerified": True,
        "idleDidNotInvokeHost": True, "manualTakeoverStoppedReceiver": True, "resumeKeptCursor": True,
    }, [reception_evidence], resumed["observedAtUnixSeconds"], replyReceipts=receipts)

    device = load(qa / "bridge-process.json")
    names = [entry["event"] for entry in run.events("bridge")]
    check(device["startedAtUnixSeconds"] >= lower, "Bridge 早于候选生成。")
    joined = run.joined()
    check(joined["identity"]["agent"]["agentId"] == presence["entry"]["agent"]["agentId"], "在场人物与验收身份不一致。")
    if run.device_mode() == "reused":
        check("ready" in names and "authorization_required" not in names, "长期验收设备没有用已保存的授权直接就绪。")
        fresh = load(qa / "device-mode.json")["record"]["freshAuthorization"]
        first_evidence = evidence("usability-evidence-first-device.json", {
            "version": metadata["version"], "observedAtUnixSeconds": after["observedAtUnixSeconds"],
            "environment": "Long-lived acceptance device profile on the local Windows computer",
            "deviceMode": "reused", "freshAuthorization": fresh,
            "bridgeExecutableSha256": digest(Path(device["executable"])), "deviceFlowEvents": names,
            "authorizationRestored": True, "bridgeConnected": True, "agentJoined": True, "actualHostRepliesVerified": 2,
        })
        first = report("first-device", {key: True for key in sorted(release_acceptance.REUSED_DEVICE_CHECKS)},
                       [first_evidence], after["observedAtUnixSeconds"], deviceMode="reused", freshAuthorization=fresh)
    else:
        check(all(name in names for name in ("authorization_required", "device_authorized", "ready")), "设备授权事件不完整。")
        check(names.index("device_authorized") < names.index("ready")
              and names[: names.index("device_authorized")].count("authorization_required") >= 1, "设备授权顺序不对。")
        first_evidence = evidence("usability-evidence-first-device.json", {
            "version": metadata["version"], "observedAtUnixSeconds": after["observedAtUnixSeconds"],
            "environment": "New isolated device profile on the local Windows computer; not a second physical computer",
            "deviceMode": "fresh", "initialProfileEmpty": True,
            "bridgeExecutableSha256": digest(Path(device["executable"])),
            "deviceFlowEvents": names, "authorizationCompleted": True, "bridgeConnected": True,
            "agentJoined": True, "actualHostRepliesVerified": 2,
        })
        first = report("first-device", {key: True for key in
                                        ["authorizationCompleted", "bridgeConnected", "agentJoined", "replyVerified"]},
                       [first_evidence], after["observedAtUnixSeconds"], deviceMode="fresh")

    upgrade = load(work / "usability-evidence-upgrade.json")
    installed = load(work / "installed-verification.json")
    baseline = load(work / "upgrade-baseline.json")
    check(upgrade["previousVersion"] == installed["previousVersion"] == baseline["version"] != metadata["version"],
          "升级验收的旧版本不一致。")
    check(upgrade["currentVersion"] == installed["version"] == metadata["version"], "升级验收不是当前候选。")
    check(installed["installerExitCode"] == 0 and installed["runtimeHashesMatched"], "安装器或运行时摘要未通过。")
    if run.migration:
        check(upgrade.get("upgradeMode") == "server-migration" and upgrade.get("serverMigration") == run.migration,
              "升级记录不是登记的服务器迁移。")
        check(all(upgrade.get(key) is True for key in ["runtimeHashesMatched", "previousLoginNotReused",
                                                        "signedInToNewServer", "bridgeReady"]),
              "服务器迁移后没有丢开旧登录并在新服务器上登录就绪。")
        upgraded = report("upgrade", {key: True for key in sorted(release_acceptance.MIGRATED_UPGRADE_CHECKS)},
                          [work / "usability-evidence-upgrade.json"], upgrade["observedAtUnixSeconds"],
                          upgradeMode="server-migration", serverMigration=run.migration)
    else:
        check(upgrade["previousPendingCount"] > 0 and not upgrade["acknowledgedDuringVerification"],
              "升级前没有待确认投递。")
        check(all(upgrade[key] for key in ["runtimeHashesMatched", "loginRestored", "identityPreserved",
                                           "roomPreserved", "pendingDeliveryPreserved"]), "升级后状态未全部保留。")
        upgraded = report("upgrade", {key: True for key in
                                      ["installedPreviousVersion", "upgradedToCandidate", "loginRestored",
                                       "identityPreserved", "pendingDeliveryPreserved"]},
                          [work / "usability-evidence-upgrade.json"], upgrade["observedAtUnixSeconds"])
    release_acceptance.assemble(run.candidate, [first, upgraded, continuous])
    release_acceptance.verify(run.candidate, metadata["version"], metadata["revision"])
    # release_flow looks for the index next to the reports as well as in the candidate directory.
    (work / "release-usability-acceptance.json").write_bytes((run.candidate / "release-usability-acceptance.json").read_bytes())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--work", type=Path, required=True, help="release directory with the verified candidate")
    parser.add_argument("command", choices=["baseline", "upgrade", "start", "run", "cleanup", "status"])
    options = parser.parse_args(argv)
    try:
        acceptance = Acceptance(options.work)
        if options.command == "baseline":
            return acceptance.baseline()
        if options.command == "upgrade":
            return acceptance.upgrade()
        if options.command == "start":
            return acceptance.start()
        if options.command == "run":
            return acceptance.run()
        if options.command == "cleanup":
            acceptance.cleanup()
            return 0
        return acceptance.status()
    except ReleaseFailure as failure:
        print(f"验收停止：{failure}", file=sys.stderr)
        print("检查 fresh-device-qa 里的记录后重跑同一命令；需要撤销测试授权时运行 cleanup。", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
