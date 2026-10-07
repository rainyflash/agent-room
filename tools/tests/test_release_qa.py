from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

from tools import release, release_acceptance, release_qa

VERSION = "0.1.0-alpha.41"
REVISION = "b" * 40


def reply(event: str, submission: str) -> dict:
    return {"type": "delivery", "record": {"stage": "replied", "eventId": event, "failure": None,
                                           "replyEventId": f"$reply-{event[1:]}", "submissionId": submission}}


def running(event: str) -> dict:
    return {"type": "delivery", "record": {"stage": "running", "eventId": event}}


SUBMISSIONS = ["0198b601-77a1-7bb8-83eb-000000000001", "0198b601-77a1-7bb8-83eb-000000000002"]
EVENTS = ["$message-one", "$message-two"]


class ReleaseQaHelpers(unittest.TestCase):
    def test_label_and_slug_follow_the_version(self):
        self.assertEqual(release_qa.release_label(VERSION), "Alpha 41")
        self.assertEqual(release_qa.release_slug(VERSION), "alpha41")
        self.assertEqual(release_qa.release_label("1.2.3"), "1.2.3")

    def test_upgrade_uses_the_windows_installer_when_the_candidate_also_has_a_mac_image(self):
        mac = {"kind": "installer", "name": "installer", "platform": "darwin-aarch64"}
        windows = {"kind": "installer", "name": "installer", "platform": "windows-x86_64"}
        desktop = {"kind": "desktop", "name": "desktop", "platform": "windows-x86_64"}
        self.assertIs(release_qa.windows_installer({"artifacts": [mac, desktop, windows]}), windows)
        for artifacts in ([mac, desktop], [windows, dict(windows)]):
            with self.assertRaisesRegex(release_qa.ReleaseFailure, "Windows 安装器"):
                release_qa.windows_installer({"artifacts": artifacts})

    def test_desktop_relaunch_matches_the_installed_path_in_either_slash_style(self):
        script = release_qa.relaunch_script("C:/Users/o'neil/AppData/Local/Agent Room/agent-room-desktop.exe", 14222)
        self.assertIn(r"'C:\Users\o''neil\AppData\Local\Agent Room\agent-room-desktop.exe'", script)
        self.assertIn("[IO.Path]::GetFullPath($_.Path) -eq $app", script)
        self.assertIn("'--remote-debugging-port=14222'", script)
        self.assertNotIn('"', script)
        plain = release_qa.relaunch_script("C:/Agent Room/agent-room-desktop.exe", None)
        self.assertNotIn("remote-debugging-port", plain)
        self.assertIn("Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", plain)

    def test_liveness_check_answers_for_exited_processes(self):
        script = release_qa.liveness_script(4242, r"C:\qa\o'k.exe")
        self.assertIn(r"'C:\qa\o''k.exe'", script)
        self.assertTrue(script.endswith("exit 0"))

    @unittest.skipUnless(sys.platform == "win32", "PowerShell process checks run on the Windows release workstation")
    def test_liveness_check_against_real_processes(self):
        shell = shutil.which("powershell")

        def check(pid: int, executable: str) -> bool:
            script = release_qa.liveness_script(pid, executable)
            return subprocess.check_output([shell, "-NoProfile", "-Command", script], encoding="utf-8").strip() == "yes"

        child = subprocess.Popen([shell, "-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
        try:
            self.assertTrue(check(child.pid, shell.replace("\\", "/")))
        finally:
            child.kill()
            child.wait()
        self.assertFalse(check(child.pid, shell))

    def test_device_decision_reuses_the_record_only_when_allowed_and_holds_for_the_release(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
            acceptance.work, acceptance.qa = work, work / "fresh-device-qa"
            acceptance.qa.mkdir()
            acceptance.metadata = {"revision": "d" * 40}
            acceptance.host_type, acceptance.label, acceptance.slug = "claude_code", "Alpha 43", "alpha43"
            record = {"dataDir": str(work / "device"), "service": "svc", "label": "Long-lived", "profileId": "p",
                      "agentId": "a", "agentName": "发布验收 Claude Code",
                      "freshAuthorization": {"version": "0.1.0", "revision": "c" * 40,
                                             "capturedAtUnixSeconds": int(time.time())}}
            acceptance.device_record_path = work / "acceptance-device.json"
            acceptance.device_record_path.write_text(json.dumps(record), encoding="utf-8")
            with mock.patch.object(release_qa.release_acceptance, "reuse_blocker", return_value="too old"):
                self.assertEqual(acceptance.decide_device(), {"mode": "fresh", "reason": "too old"})
            (acceptance.qa / "device-mode.json").unlink()
            with mock.patch.object(release_qa.release_acceptance, "reuse_blocker", return_value=None):
                decision = acceptance.decide_device()
            self.assertEqual(decision["mode"], "reused")
            with mock.patch.object(release_qa.release_acceptance, "reuse_blocker", return_value="changed"):
                self.assertEqual(acceptance.decide_device()["mode"], "reused")
            acceptance.apply_device(decision)
            self.assertEqual((acceptance.service, acceptance.agent_name), ("svc", "发布验收 Claude Code"))
            acceptance.apply_device({"mode": "fresh"})
            self.assertEqual(acceptance.service, "agent-room.alpha43.acceptance.fresh-device")
            self.assertEqual(acceptance.agent_name, "发布验收 Claude Code")

    def test_reused_device_rebuilds_its_invitation_from_the_recorded_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
            acceptance.work, acceptance.qa = work, work / "fresh-device-qa"
            acceptance.qa.mkdir()
            record = {"profileId": "p", "agentId": "a", "agentName": "发布验收 Claude Code"}
            (acceptance.qa / "device-mode.json").write_text(
                json.dumps({"mode": "reused", "record": record}), encoding="utf-8")
            (acceptance.qa / "joined.private.json").write_text(json.dumps(
                {"profileId": "p", "identity": {"roomId": "!room:example", "roomCatalogId": "catalog"}}),
                encoding="utf-8")
            self.assertEqual(acceptance.invitation(), {
                "version": 1, "sessionKey": "p", "displayName": "发布验收 Claude Code",
                "roomId": "!room:example", "catalogId": "catalog"})
            (acceptance.qa / "device-mode.json").write_text(json.dumps({"mode": "fresh"}), encoding="utf-8")
            with self.assertRaisesRegex(release_qa.ReleaseFailure, "邀请"):
                acceptance.invitation()

    def test_baseline_is_recorded_once_and_reused(self):
        with tempfile.TemporaryDirectory() as directory:
            acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
            acceptance.work = Path(directory)
            captured = []
            acceptance.upgrade_baseline = lambda: captured.append(True) or (acceptance.work / "upgrade-baseline.json").write_text("{}")
            self.assertEqual(acceptance.baseline(), 0)
            self.assertEqual(acceptance.baseline(), 0)
            self.assertEqual(captured, [True])

    def test_server_move_requires_the_old_agent_target_to_be_retired(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            data = work / "Bridge"
            acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
            acceptance.work = work
            acceptance.config = {"controlPlaneUrl": "https://api.new.example/",
                                 "upgrade": {"bridgeDataDir": str(data)}}
            old_target = b'{"schemaVersion":1}'
            (work / "upgrade-baseline.json").write_text(json.dumps(
                {"agentTargetSha256": hashlib.sha256(old_target).hexdigest()}), encoding="utf-8")
            (data / "desktop").mkdir(parents=True)
            (data / "desktop" / "agent-target.json").write_bytes(old_target)
            (data / "deployment.json").write_text(json.dumps({"controlPlaneUrl": "https://api.new.example/"}),
                                                  encoding="utf-8")
            with self.assertRaisesRegex(release.ReleaseFailure, "retired"):
                acceptance.previous_state_retired()
            (data / "retired" / "1789740000").mkdir(parents=True)
            (data / "desktop").rename(data / "retired" / "1789740000" / "desktop")
            self.assertEqual(acceptance.previous_state_retired(),
                             {"deploymentRecorded": True, "previousAgentTargetRetired": True})
            (data / "deployment.json").write_text(json.dumps({"controlPlaneUrl": "https://api.old.example/"}),
                                                  encoding="utf-8")
            with self.assertRaisesRegex(release.ReleaseFailure, "新服务器"):
                acceptance.previous_state_retired()

    def test_controls_match_either_language_exactly(self):
        self.assertEqual(release_qa.js_name("mention"), "/^(提及 Agent|Mention an agent)$/")
        self.assertEqual(release_qa.js_name("log"), "/^(房间对话|Conversation)$/")

    def test_fill_rejects_missing_and_leftover_placeholders(self):
        self.assertEqual(release_qa.fill("a @@X@@ b", {"X": "1"}), "a 1 b")
        with self.assertRaises(ValueError):
            release_qa.fill("a @@X@@ @@Y@@", {"X": "1"})
        with self.assertRaises(ValueError):
            release_qa.fill("a", {"X": "1"})

    def test_message_with_attachment_is_fully_filled(self):
        source = release_qa.send_message_js(2, "第二条", "@agent:example", Path("C:/qa/alpha41-attachment.txt"))
        self.assertNotRegex(source, r"@@[A-Z_]+@@")
        self.assertIn("setInputFiles", source)
        self.assertIn("attachmentSent: true", source)
        self.assertIn(release_qa.js_name("send"), source)
        without = release_qa.send_message_js(1, "第一条", "@agent:example", None)
        self.assertNotIn("setInputFiles", without)
        self.assertIn("attachmentSent: false", without)

    def test_persona_namespace_matches_the_bridge(self):
        # The same vector is pinned by the Bridge test 人物存储命名空间与发布验收工具算法一致.
        self.assertEqual(
            release_qa.host_storage_service("agent-room.alpha52.acceptance.fresh-device",
                                            "01a0e602-f68e-7ee2-86ab-71276e653172"),
            "dev.agent-room.host.MrRUZtsV2yGnp2fG9FiLVNpZRQkzj6AkIQSOlJ491ts.v1")

    def test_retiring_a_device_selects_only_its_own_and_its_personas_credentials(self):
        old = "agent-room.alpha52.acceptance.fresh-device"
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)
            (data / "host-agents" / "01a0e602-f68e-7ee2-86ab-71276e653172").mkdir(parents=True)
            persona = release_qa.host_storage_service(old, "01a0e602-f68e-7ee2-86ab-71276e653172")
            desktop_persona = release_qa.host_storage_service("dev.agent-room.bridge", "01a0e602-f68e-7ee2-86ab-71276e653172")
            targets = [
                f"device-session-v1.{old}", f"bridge-ipc-shared-secret-v1.{persona}",
                "device-session-v1.dev.agent-room.bridge", f"device-session-v1.{desktop_persona}",
                "device-session-v1.agent-room.alpha53.acceptance.fresh-device",
                f"Some Other App.{old}", f"device-session-v1.{old}.extra",
            ]
            record = {"service": old, "dataDir": str(data)}
            self.assertEqual(release_qa.retired_device_targets(record, targets),
                             [f"device-session-v1.{old}", f"bridge-ipc-shared-secret-v1.{persona}"])
            with self.assertRaises(release.ReleaseFailure):
                release_qa.retired_device_targets({"service": "dev.agent-room.bridge", "dataDir": str(data)}, targets)

    def test_retiring_deletes_one_by_one_and_only_warns_on_failure(self):
        old = "agent-room.alpha52.acceptance.fresh-device"
        targets = [f"device-session-v1.{old}", f"matrix-store-passphrase-v1.{old}", "device-session-v1.dev.agent-room.bridge"]
        deleted: list[str] = []
        with mock.patch.object(release_qa.sys, "platform", "win32"), \
                mock.patch.object(release_qa, "windows_generic_credentials", return_value=targets), \
                mock.patch.object(release_qa, "delete_windows_credential",
                                  side_effect=lambda target: deleted.append(target) or True):
            release_qa.retire_device_credentials({"service": old, "dataDir": "missing"})
        self.assertEqual(deleted, targets[:2])
        with mock.patch.object(release_qa.sys, "platform", "win32"), \
                mock.patch.object(release_qa, "windows_generic_credentials", side_effect=OSError(5, "denied")):
            release_qa.retire_device_credentials({"service": old, "dataDir": "missing"})

    @unittest.skipIf(shutil.which("node") is None, "node is required to run the page function")
    def test_reply_check_only_counts_this_runs_two_newest_replies(self):
        # Earlier releases' QA Agent has the same name, and the room fills in their replies on open.
        source = release_qa.verify_replies_js("发布验收 Claude Code", "ALPHA53-ABC")
        harness = r"""
const check = (0, eval)('(' + require('fs').readFileSync(0, 'utf8').trim() + ')');
const texts = JSON.parse(process.argv[1]);
function locator(items) {
  return {
    locator: () => locator(items),
    filter: ({hasText}) => locator(items.filter((text) => text.includes(hasText))),
    count: async () => items.length,
    nth: (index) => locator(items.slice(index, index + 1)),
    innerText: async () => {
      if (items.length !== 1) throw new Error('strict mode violation: ' + items.length);
      return items[0];
    },
    waitFor: async () => {
      if (items.length !== 1) throw new Error('strict mode violation: ' + items.length);
    },
  };
}
check({getByRole: () => locator(texts)}).then(
  (result) => { process.stdout.write(JSON.stringify(result)); },
  (error) => { process.stdout.write('ERROR ' + error.message); process.exitCode = 4; });
"""

        def run(texts):
            return subprocess.run(["node", "-e", harness, json.dumps(texts, ensure_ascii=False)], input=source,
                                  capture_output=True, text=True, encoding="utf-8")

        name = "发布验收 Claude Code"
        passed = run([f"{name} 第一条消息已收到", f"{name} ALPHA52-OLD", f"{name} 第一条消息已收到",
                      f"{name} 附件里写着 ALPHA53-ABC"])
        self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)
        self.assertEqual(json.loads(passed.stdout)["visibleAgentReplies"], 2)
        for texts in ([f"{name} 别的回复", f"{name} ALPHA53-ABC"],
                      [f"{name} ALPHA53-ABC", f"{name} 第一条消息已收到"],
                      [f"{name} ALPHA53-ABC"]):
            with self.subTest(texts=texts):
                self.assertEqual(run(texts).returncode, 4)

    @unittest.skipIf(shutil.which("node") is None, "node is required to parse the page functions")
    def test_every_page_function_is_valid_javascript(self):
        identity = {"agent": {"agentId": "a", "matrixUserId": "@a:example"}, "instanceId": "i", "roomCatalogId": "c"}
        target = {"agentId": "a", "catalogId": "c", "nextDeviceId": None}
        sources = [
            release_qa.create_grant_js(VERSION, release_qa.grant_payload("g", identity)),
            release_qa.send_message_js(1, "一", "@a:example", None),
            release_qa.send_message_js(2, "二", "@a:example", Path("C:/qa/alpha41-attachment.txt")),
            release_qa.reception_js("read", target),
            release_qa.reception_js("takeover", target),
            release_qa.verify_replies_js("Alpha 41 实机验收 Claude Code", "ALPHA41-ABC"),
            release_qa.revoke_grant_js("g"),
            release_qa.goto_room_js("http://tauri.localhost/lobby/c/instance/r?view=conversation"),
            release_qa.native_session_js("http://tauri.localhost", VERSION),
            release_qa.migrated_session_js("http://tauri.localhost", VERSION),
        ]
        check = ("const s = require('fs').readFileSync(0, 'utf8');"
                 "if (typeof (0, eval)('(' + s.trim() + ')') !== 'function') process.exit(3);")
        for source in sources:
            with self.subTest(source=source[:60]):
                result = subprocess.run(["node", "-e", check], input=source, capture_output=True, text=True, encoding="utf-8")
                self.assertEqual(result.returncode, 0, result.stderr)

    # A fake desktop page: `desktop_control_plane_request` decodes the frame the way control_plane_proxy.rs
    # does and answers from argv; WebView fetch throws, because the desktop's login lives only in the native
    # layer since #325.
    NATIVE_HARNESS = r"""
const run = (0, eval)('(' + require('fs').readFileSync(0, 'utf8').trim() + ')');
const answers = JSON.parse(process.argv[1]);
const requests = [];
globalThis.fetch = () => { throw new Error('WebView fetch carries no login'); };
globalThis.location = {origin: 'http://tauri.localhost'};
globalThis.window = {__TAURI_INTERNALS__: {invoke: async (command, frame) => {
  if (command === 'desktop_runtime_snapshot') return answers.runtime;
  if (command !== 'desktop_control_plane_request') throw new Error('unexpected command ' + command);
  const length = new DataView(frame.buffer, frame.byteOffset, frame.byteLength).getUint32(0);
  const head = JSON.parse(new TextDecoder().decode(frame.subarray(4, 4 + length)));
  const body = new TextDecoder().decode(frame.subarray(4 + length));
  requests.push({...head, body: body === '' ? null : JSON.parse(body)});
  const answer = answers[head.method + ' ' + head.path];
  if (!answer) throw new Error('unexpected request ' + head.method + ' ' + head.path);
  const responseHead = new TextEncoder().encode(JSON.stringify({status: answer.status, headers: []}));
  const responseBody = new TextEncoder().encode(answer.body === null ? '' : JSON.stringify(answer.body));
  const out = new Uint8Array(4 + responseHead.byteLength + responseBody.byteLength);
  new DataView(out.buffer).setUint32(0, responseHead.byteLength);
  out.set(responseHead, 4);
  out.set(responseBody, 4 + responseHead.byteLength);
  // The custom protocol answers with an ArrayBuffer, the message-channel fallback with an array of numbers.
  return requests.length % 2 === 1 ? out.buffer : Array.from(out);
}}};
run({evaluate: (fn, arg) => fn(arg)}).then(
  (result) => { process.stdout.write(JSON.stringify({result, requests})); },
  (error) => { process.stdout.write('ERROR ' + error.message); process.exitCode = 4; });
"""

    def run_native(self, source: str, answers: dict) -> tuple[int, str]:
        result = subprocess.run(["node", "-e", self.NATIVE_HARNESS, json.dumps(answers)], input=source,
                                capture_output=True, text=True, encoding="utf-8")
        return result.returncode, result.stdout + result.stderr

    @unittest.skipIf(shutil.which("node") is None, "node is required to run the page functions")
    def test_control_plane_requests_go_through_the_native_layer(self):
        identity = {"agent": {"agentId": "a", "matrixUserId": "@a:example"}, "instanceId": "i", "roomCatalogId": "c"}
        payload = release_qa.grant_payload("g", identity)
        code, out = self.run_native(release_qa.create_grant_js(VERSION, payload), {
            "GET health/ready": {"status": 200, "body": {"version": VERSION}},
            "GET auth/session": {"status": 200, "body": {"principalId": "owner"}},
            "POST automation-grants": {"status": 201, "body": {"grantId": "g", "status": "active"}},
        })
        self.assertEqual(code, 0, out)
        output = json.loads(out)
        self.assertEqual(output["result"]["principalId"], "owner")
        self.assertEqual(output["requests"], [
            {"method": "GET", "path": "health/ready", "headers": [], "body": None},
            {"method": "GET", "path": "auth/session", "headers": [], "body": None},
            {"method": "POST", "path": "automation-grants",
             "headers": [["Idempotency-Key", "g"], ["content-type", "application/json"]], "body": payload["input"]},
        ])

        target = {"agentId": "a", "catalogId": "c", "nextDeviceId": None}
        code, out = self.run_native(release_qa.reception_js("takeover", target), {
            "POST receptions/transfer": {"status": 200, "body": {"agentId": "a", "catalogId": "c", "status": "idle"}},
        })
        self.assertEqual(code, 0, out)
        self.assertEqual(json.loads(out)["requests"][0]["body"], target)

        code, out = self.run_native(release_qa.revoke_grant_js("g"),
                                    {"DELETE automation-grants/g": {"status": 204, "body": None}})
        self.assertEqual((code, json.loads(out)["result"]["revoked"]), (0, True), out)

    @unittest.skipIf(shutil.which("node") is None, "node is required to run the page functions")
    def test_restored_login_is_checked_through_the_native_layer(self):
        runtime = {"currentVersion": VERSION, "updatesConfigured": True,
                   "bridge": {"lifecycle": {"phase": "authorized"}, "authorization": None}}
        source = release_qa.native_session_js("http://tauri.localhost", VERSION)
        code, out = self.run_native(source, {
            "runtime": runtime, "GET auth/session": {"status": 200, "body": {"principalId": "owner"}}})
        self.assertEqual(code, 0, out)
        self.assertEqual((json.loads(out)["result"]["loginRestored"], json.loads(out)["result"]["httpSessionStatus"]),
                         (True, 200))
        code, out = self.run_native(source, {
            "runtime": runtime, "GET auth/session": {"status": 401, "body": {"code": "auth.session_required"}}})
        self.assertEqual(code, 4, out)
        self.assertIn('"loginRestored":false', out)

    @unittest.skipIf(shutil.which("node") is None, "node is required to run the page function")
    def test_a_message_already_shown_is_not_sent_again(self):
        source = release_qa.send_message_js(1, "第一条", "@agent:example", None)
        harness = r"""
const send = (0, eval)('(' + require('fs').readFileSync(0, 'utf8').trim() + ')');
const state = JSON.parse(process.argv[1]);
const actions = [];
const sent = {
  locator: () => sent,
  filter: () => sent,
  count: async () => state.shown,
  first: () => ({waitFor: async () => { if (state.shown === 0) throw new Error('Timeout 45000ms exceeded'); }}),
};
const page = {
  getByRole: (role) => ({
    log: sent,
    textbox: {inputValue: async () => state.draft, fill: async (text) => { actions.push('fill ' + text); }},
    combobox: {selectOption: async (value) => { actions.push('mention ' + value); }},
    button: {click: async () => { actions.push('send'); state.shown += 1; }},
  })[role],
};
send(page).then(
  (result) => { process.stdout.write(JSON.stringify({result, actions})); },
  (error) => { process.stdout.write('ERROR ' + error.message); process.exitCode = 4; });
"""

        def run(state):
            return subprocess.run(["node", "-e", harness, json.dumps(state)], input=source,
                                  capture_output=True, text=True, encoding="utf-8")

        fresh = run({"shown": 0, "draft": ""})
        self.assertEqual(fresh.returncode, 0, fresh.stdout + fresh.stderr)
        self.assertEqual(json.loads(fresh.stdout)["actions"], ["mention @agent:example", "fill 第一条", "send"])
        shown = run({"shown": 1, "draft": ""})
        self.assertEqual(shown.returncode, 0, shown.stdout + shown.stderr)
        self.assertEqual(json.loads(shown.stdout)["actions"], [])
        self.assertIn("Preserve the existing draft", run({"shown": 0, "draft": "别动我的草稿"}).stdout)
        self.assertIn("not shown exactly once", run({"shown": 2, "draft": ""}).stdout)

    def test_grant_is_bounded_to_one_instance_and_replies(self):
        payload = release_qa.grant_payload("id", {"agent": {"agentId": "a"}, "instanceId": "i", "roomCatalogId": "c"})
        grant = payload["input"]
        self.assertEqual((grant["agentInstanceId"], grant["messageKinds"]), ("i", ["reply"]))
        self.assertEqual((grant["lifetimeSeconds"], grant["maxTotalMessages"]), (3600, 5))
        self.assertTrue(grant["requiresRiskScan"])


class ConfirmDeliveryTests(unittest.TestCase):
    """Since #325 publication is out of CDP's sight: the receiver's decrypted copy stands in for the receipt."""

    def acceptance(self, event_ids: list[str], shown: dict[str, list[dict]]):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        qa = Path(temporary.name)
        (qa / "authorization.private.json").write_text(json.dumps({"principalId": "owner"}), encoding="utf-8")
        (qa / "joined.private.json").write_text(json.dumps({"profileId": "profile"}), encoding="utf-8")
        acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
        acceptance.qa = qa
        acceptance.room_id = "!room:example"
        acceptance.label = "Alpha 41"
        acceptance.canary_name = "alpha41-attachment.txt"
        events = [{"ok": True, "data": {"type": "delivery", "record": {"stage": stage, "eventId": event}}}
                  for event in event_ids for stage in ("received", "running", "replied")]
        acceptance.receiver_events = lambda: events
        acceptance.cli = mock.Mock(side_effect=lambda *args, **_: {"messages": shown.get(args[-1], [])})
        return acceptance

    @staticmethod
    def message(number: int, event: str, **changes):
        message = {"eventId": event, "roomId": "!room:example", "actor": {"principalId": "owner"},
                   "conversation": {"text": release_qa.message_texts("Alpha 41")[number], "mentions": []},
                   "mentionsMe": True, "content": {"contentId": f"content-{event}"}}
        if number == 2:
            message["conversation"]["attachmentName"] = "alpha41-attachment.txt"
        message.update(changes)
        return message

    def test_records_the_decrypted_message_the_receiver_got(self):
        first = self.acceptance(["$one"], {"$one": [self.message(1, "$one")]}).confirm_delivery(1)
        self.assertEqual((first["eventId"], first["contentId"], first["attachmentSent"]),
                         ("$one", "content-$one", False))
        acceptance = self.acceptance(["$one", "$two"], {"$two": [self.message(2, "$two")]})
        second = acceptance.confirm_delivery(2)
        self.assertEqual((second["eventId"], second["contentId"], second["attachmentSent"]),
                         ("$two", "content-$two", True))
        acceptance.cli.assert_called_with("--profile", "profile", "show", "--id", "$two")

    def test_rejects_a_copy_that_differs_from_what_was_sent(self):
        wrong = {
            "text": {"conversation": {"text": "别的话", "mentions": []}},
            "room": {"roomId": "!other:example"},
            "sender": {"actor": {"principalId": "someone-else"}},
            "mention": {"mentionsMe": False},
            "attachment": {"conversation": {"text": release_qa.message_texts("Alpha 41")[1], "mentions": [],
                                            "attachmentName": "alpha41-attachment.txt"}},
        }
        for name, changes in wrong.items():
            with self.subTest(name):
                acceptance = self.acceptance(["$one"], {"$one": [self.message(1, "$one", **changes)]})
                with self.assertRaises(release_qa.ReleaseFailure):
                    acceptance.confirm_delivery(1)
        missing = self.message(2, "$two")
        del missing["conversation"]["attachmentName"]
        acceptance = self.acceptance(["$one", "$two"], {"$two": [missing]})
        with self.assertRaises(release_qa.ReleaseFailure):
            acceptance.confirm_delivery(2)

    def test_an_extra_delivery_or_none_at_all_fails(self):
        acceptance = self.acceptance(["$one", "$stray"], {"$stray": [self.message(1, "$stray")]})
        with self.assertRaisesRegex(release_qa.ReleaseFailure, "验收以外"):
            acceptance.confirm_delivery(1)
        acceptance = self.acceptance([], {})
        with mock.patch.object(release_qa.time, "time", side_effect=[0, 0, 91]), \
                mock.patch.object(release_qa.time, "sleep"):
            with self.assertRaisesRegex(release_qa.ReleaseFailure, "90 秒内"):
                acceptance.confirm_delivery(1)

    def test_a_message_the_receiver_already_has_is_not_sent_again(self):
        acceptance = self.acceptance(["$one"], {"$one": [self.message(1, "$one")]})
        acceptance.page = mock.Mock()
        acceptance.send(1)
        acceptance.page.assert_not_called()
        self.assertEqual(json.loads((acceptance.qa / "incoming-1.json").read_text(encoding="utf-8"))["eventId"], "$one")


class AssembleReportsTests(unittest.TestCase):
    """Synthetic records only: these prove the checks, never a release."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.qa = self.work / "fresh-device-qa"
        self.candidate = self.work / "candidate"
        self.qa.mkdir()
        self.candidate.mkdir()
        self.now = int(time.time())
        (self.candidate / "release.signed.json").write_text("synthetic", encoding="utf-8")
        signed = release.sha256_file(self.candidate / "release.signed.json")
        self.write(self.candidate / "release-metadata.json", {
            "version": VERSION, "revision": REVISION, "sequence": 41, "publishedAtUnixSeconds": self.now - 3600})
        self.write(self.candidate / "release.json", {"version": VERSION, "artifacts": []})
        self.write(self.work / "verified-candidate.json", {
            "version": VERSION, "revision": REVISION, "signedManifestSha256": signed,
            **{key: True for key in ["independentRootSignaturePassed", "sigstorePassed", "certificateRevisionPassed",
                                     "tauriSignaturePassed", "ciInstallerAcceptanceMatched",
                                     "artifactDigestsAndSbomPassed", "remoteIndexDigestsPassed"]}})
        self.write(self.work / "ci-verification.json", {"revision": REVISION, "allRequiredJobsPassed": True})
        host = self.work / "claude.exe"
        host.write_text("host", encoding="utf-8")
        self.write(self.work / "qa-host.json", {"hostType": "claude_code", "taskId": "task", "realHostInitialized": True,
                                                "executable": str(host), "workspace": str(self.work / "qa-workspace")})
        self.write(self.work / "release-qa.json", {
            "controlPlaneUrl": "https://api.example/", "matrixBaseUrl": "https://matrix.example/",
            "oidcIssuerUrl": "https://id.example/realms/agent-room", "oidcDeviceClientId": "agent-room-bridge",
            "room": {"roomId": "!room:example", "catalogId": "catalog"},
            "desktop": {"executable": "C:/desktop.exe"}})
        self.write(self.work / "installed-verification.json", {
            "version": VERSION, "previousVersion": "0.1.0-alpha.40", "installerExitCode": 0, "runtimeHashesMatched": True})
        self.write(self.work / "upgrade-baseline.json", {"version": "0.1.0-alpha.40"})
        self.write(self.work / "usability-evidence-upgrade.json", {
            "previousVersion": "0.1.0-alpha.40", "currentVersion": VERSION, "previousPendingCount": 3,
            "acknowledgedDuringVerification": False, "runtimeHashesMatched": True, "loginRestored": True,
            "identityPreserved": True, "roomPreserved": True, "pendingDeliveryPreserved": True,
            "observedAtUnixSeconds": self.now - 1800})
        bridge = self.work / "bridge.exe"
        bridge.write_text("bridge", encoding="utf-8")
        self.write(self.qa / "bridge-process.json", {"pid": 1, "startedAtUnixSeconds": self.now - 1500, "executable": str(bridge)})
        (self.qa / "bridge.private.jsonl").write_text("\n".join(json.dumps({"event": name}) for name in [
            "authorization_required", "authorization_required", "device_authorized", "ready"]) + "\n", encoding="utf-8")
        self.write(self.qa / "joined.private.json", {"profileId": "p", "identity": {"agent": {"agentId": "agent"}}})
        self.write(self.qa / "presence-during-idle.json", {"entry": {"agent": {"agentId": "agent"},
                                                                     "lifecycle": {"reception": "waiting", "connection": "online"}}})
        for number, event in enumerate(EVENTS, start=1):
            self.write(self.qa / f"incoming-{number}.json", {"eventId": event, "attachmentSent": number == 2})
        self.write(self.qa / "verified-replies.json", {"visibleAgentReplies": 2, "firstReplyTextConfirmed": True,
                                                       "attachmentCanaryConfirmed": True})
        handled = [running(EVENTS[0]), reply(EVENTS[0], SUBMISSIONS[0]), running(EVENTS[1]), reply(EVENTS[1], SUBMISSIONS[1])]
        self.observe("idle-before", self.now - 700, handled, alive=True, checkpoint="c1", execution="run-1")
        self.observe("idle-after", self.now - 630, handled, alive=True, checkpoint="c1", execution="run-1")
        self.observe("after-takeover", self.now - 600, [*handled, {"type": "stopped"}], alive=False,
                     checkpoint="c1", execution=None, enabled=False)
        self.observe("after-resume", self.now - 500, [*handled, {"type": "stopped"}, {"type": "ready"}], alive=True,
                     checkpoint="c1", execution="run-2")
        self.write(self.qa / "reception-stopped.json", {"record": {"status": "idle", "progress": {"after": "x"}, "runId": "r1"}})
        self.write(self.qa / "reception-resumed.json", {"record": {"status": "active", "progress": {"after": "x"}, "runId": "r2"}})

    @staticmethod
    def write(path: Path, value: dict) -> None:
        path.write_text(json.dumps(value), encoding="utf-8")

    def observe(self, label, at, events, *, alive, checkpoint, execution, enabled=True):
        self.write(self.qa / f"observation-{label}.json", {
            "observedAtUnixSeconds": at, "receiverProcessAlive": alive, "events": events, "errors": [],
            "partialLastLine": False, "state": {"enabled": enabled, "execution": execution, "checkpoint": checkpoint}})

    def assemble(self):
        release_qa.assemble_reports(release_qa.Acceptance(self.work))

    def test_observed_run_assembles_into_verifiable_candidate_evidence(self):
        self.assemble()
        release_acceptance.verify(self.candidate, VERSION, REVISION)
        continuous = json.loads((self.work / "release-usability-continuous-reception.json").read_text(encoding="utf-8"))
        self.assertEqual([item["submissionId"] for item in continuous["replyReceipts"]], SUBMISSIONS)
        self.assertTrue((self.work / "release-usability-acceptance.json").exists())

    def test_short_idle_is_not_accepted(self):
        self.observe("idle-after", self.now - 680, json.loads(
            (self.qa / "observation-idle-before.json").read_text(encoding="utf-8"))["events"],
            alive=True, checkpoint="c1", execution="run-1")
        with self.assertRaisesRegex(release.ReleaseFailure, "60"):
            self.assemble()

    def test_resume_that_replays_messages_is_not_accepted(self):
        before = json.loads((self.qa / "observation-after-resume.json").read_text(encoding="utf-8"))
        self.observe("after-resume", self.now - 500, [*before["events"], running(EVENTS[0])], alive=True,
                     checkpoint="c1", execution="run-2")
        with self.assertRaises(release.ReleaseFailure):
            self.assemble()

    def test_resume_on_the_same_run_is_not_accepted(self):
        self.write(self.qa / "reception-resumed.json", {"record": {"status": "active", "progress": {"after": "x"}, "runId": "r1"}})
        with self.assertRaises(release.ReleaseFailure):
            self.assemble()

    def test_missing_device_approval_is_not_accepted(self):
        (self.qa / "bridge.private.jsonl").write_text(json.dumps({"event": "authorization_required"}) + "\n", encoding="utf-8")
        with self.assertRaises(release.ReleaseFailure):
            self.assemble()

    MOVE = {"from": "old.example", "to": "new.example"}

    def migrated_upgrade(self, **fields):
        self.write(self.work / "usability-evidence-upgrade.json", {
            "previousVersion": "0.1.0-alpha.40", "currentVersion": VERSION, "runtimeHashesMatched": True,
            "upgradeMode": "server-migration", "serverMigration": self.MOVE, "previousLoginNotReused": True,
            "previousAgentTargetRetired": True, "signedInToNewServer": True, "bridgeReady": True,
            "observedAtUnixSeconds": self.now - 1800, **fields})

    def test_server_move_upgrade_assembles_without_claiming_kept_state(self):
        self.migrated_upgrade()
        with mock.patch.dict(release_acceptance.SERVER_MIGRATIONS, {VERSION: self.MOVE}):
            self.assemble()
            release_acceptance.verify(self.candidate, VERSION, REVISION)
        upgrade = json.loads((self.work / "release-usability-upgrade.json").read_text(encoding="utf-8"))
        self.assertEqual(upgrade["upgradeMode"], "server-migration")
        self.assertEqual(set(upgrade["checks"]), release_acceptance.MIGRATED_UPGRADE_CHECKS)

    def test_server_move_upgrade_needs_a_new_sign_in(self):
        self.migrated_upgrade(signedInToNewServer=False)
        with mock.patch.dict(release_acceptance.SERVER_MIGRATIONS, {VERSION: self.MOVE}):
            with self.assertRaisesRegex(release.ReleaseFailure, "新服务器"):
                self.assemble()


if __name__ == "__main__":
    unittest.main()
