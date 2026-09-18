from __future__ import annotations

import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

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

    def test_baseline_is_recorded_once_and_reused(self):
        with tempfile.TemporaryDirectory() as directory:
            acceptance = release_qa.Acceptance.__new__(release_qa.Acceptance)
            acceptance.work = Path(directory)
            captured = []
            acceptance.upgrade_baseline = lambda: captured.append(True) or (acceptance.work / "upgrade-baseline.json").write_text("{}")
            self.assertEqual(acceptance.baseline(), 0)
            self.assertEqual(acceptance.baseline(), 0)
            self.assertEqual(captured, [True])

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
        source = release_qa.send_message_js(2, "第二条", "@agent:example", "!room:example", Path("C:/qa/alpha41-attachment.txt"))
        self.assertNotRegex(source, r"@@[A-Z_]+@@")
        self.assertIn("setInputFiles", source)
        self.assertIn("attachmentSent: true", source)
        self.assertIn(release_qa.js_name("send"), source)
        without = release_qa.send_message_js(1, "第一条", "@agent:example", "!room:example", None)
        self.assertNotIn("setInputFiles", without)
        self.assertIn("attachmentSent: false", without)

    @unittest.skipIf(shutil.which("node") is None, "node is required to parse the page functions")
    def test_every_page_function_is_valid_javascript(self):
        identity = {"agent": {"agentId": "a", "matrixUserId": "@a:example"}, "instanceId": "i", "roomCatalogId": "c"}
        target = {"agentId": "a", "catalogId": "c", "nextDeviceId": None}
        sources = [
            release_qa.create_grant_js("https://api.example", VERSION, release_qa.grant_payload("g", identity)),
            release_qa.send_message_js(1, "一", "@a:example", "!r:example", None),
            release_qa.send_message_js(2, "二", "@a:example", "!r:example", Path("C:/qa/alpha41-attachment.txt")),
            release_qa.reception_js("https://api.example", "read", target),
            release_qa.reception_js("https://api.example", "takeover", target),
            release_qa.verify_replies_js("Alpha 41 实机验收 Claude Code", "ALPHA41-ABC"),
            release_qa.revoke_grant_js("https://api.example", "g"),
            release_qa.goto_room_js("http://tauri.localhost/lobby/c/instance/r?view=conversation"),
            release_qa.native_session_js("http://tauri.localhost", "https://api.example", VERSION),
        ]
        check = ("const s = require('fs').readFileSync(0, 'utf8');"
                 "if (typeof (0, eval)('(' + s.trim() + ')') !== 'function') process.exit(3);")
        for source in sources:
            with self.subTest(source=source[:60]):
                result = subprocess.run(["node", "-e", check], input=source, capture_output=True, text=True, encoding="utf-8")
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_grant_is_bounded_to_one_instance_and_replies(self):
        payload = release_qa.grant_payload("id", {"agent": {"agentId": "a"}, "instanceId": "i", "roomCatalogId": "c"})
        grant = payload["input"]
        self.assertEqual((grant["agentInstanceId"], grant["messageKinds"]), ("i", ["reply"]))
        self.assertEqual((grant["lifetimeSeconds"], grant["maxTotalMessages"]), (3600, 5))
        self.assertTrue(grant["requiresRiskScan"])


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


if __name__ == "__main__":
    unittest.main()
