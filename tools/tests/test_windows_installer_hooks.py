from __future__ import annotations

import contextlib
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import MagicMock, call, patch
import zipfile

from tools.windows_installer_acceptance import WindowsInstallerAcceptanceFailure
from tools.windows_installer_hooks import (
    HARNESS_SCRIPT,
    HOOKS,
    IMAGE_RELEASE_DELAY_SECONDS,
    INSTALLER_TIMEOUT_SECONDS,
    ISOLATED_IMAGES,
    NSIS_ARCHIVE_ROOT,
    PINNED_FILES,
    PLACEHOLDER_SCRIPT,
    PRODUCT_IMAGES,
    SCENARIOS,
    TAURI_CLI_VERSION,
    TAURI_TEMPLATE_COMMIT,
    TEMPLATE_ENGLISH,
    TEMPLATE_UTILS,
    InstallerHooksCheckFailure,
    InstallerRun,
    PinnedFile,
    Scenario,
    Toolchain,
    build_payloads,
    directory_differences,
    expected_after,
    extract_nsis,
    harness_defines,
    install_command,
    makensis_command,
    obtain,
    pinned_mismatch,
    referenced_executables,
    rename_runtime_images,
    require_disposable_runner,
    run_while_desktop_image_is_held,
    scenario_problems,
    uninstall_command,
    verify_hooks_cover_runtime,
)


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "windows-installer-hooks.yml"
DESKTOP = ROOT / "apps" / "desktop"
TOOLCHAIN = Toolchain(Path("nsis") / "makensis.exe", Path("plugins"), Path("template"))
PREVIOUS = {name: f"previous {name}" for name in PRODUCT_IMAGES.names()}
CURRENT = {name: f"current {name}" for name in PRODUCT_IMAGES.names()}
LABELS = {
    **{digest: "上一版" for digest in PREVIOUS.values()},
    **{digest: "新版" for digest in CURRENT.values()},
}


def statements(script: str) -> list[str]:
    return [line.strip() for line in script.splitlines() if line.strip() and not line.strip().startswith(";")]


def section(script: str, name: str) -> list[str]:
    lines = statements(script)
    start = lines.index(f"Section {name}")
    return lines[start + 1 : lines.index("SectionEnd", start)]


def positions(lines: list[str], prefixes: tuple[str, ...]) -> list[int]:
    return [next(index for index, line in enumerate(lines) if line.startswith(prefix)) for prefix in prefixes]


def scenario(key: str) -> Scenario:
    return next(candidate for candidate in SCENARIOS if candidate.key == key)


def trigger_paths(workflow: str, trigger: str) -> list[str]:
    lines = workflow.splitlines()
    paths: list[str] = []
    for line in lines[lines.index(f"  {trigger}:") + 1 :]:
        if not line.startswith("    "):
            break
        item = re.fullmatch(r"      - '([^']+)'", line)
        if item:
            paths.append(item.group(1))
    return paths


class HarnessMirrorsTemplateTests(unittest.TestCase):
    def test_install_section_stops_the_runtime_before_writing_the_main_program_first(self) -> None:
        lines = section(HARNESS_SCRIPT.read_text(encoding="utf-8"), "Install")

        # 与 Tauri 2.11.1 模板的安装段同序：钩子、模板的 CheckIfAppIsRunning，然后先写主程序再写 sidecar。
        order = positions(
            lines,
            (
                "SetOutPath $INSTDIR",
                "!insertmacro NSIS_HOOK_PREINSTALL",
                '!insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"',
                'File "${HARNESS_PAYLOAD_DIR}\\${MAINBINARYNAME}.exe"',
                'File /a "/oname=${HARNESS_BRIDGE}.exe"',
                'File /a "/oname=${HARNESS_MCP}.exe"',
                'File /a "/oname=${HARNESS_CLI}.exe"',
                'WriteUninstaller "$INSTDIR\\uninstall.exe"',
            ),
        )
        self.assertEqual(order, sorted(order))
        self.assertEqual(order[0], 0)
        self.assertEqual(sum(line.startswith("File ") for line in lines), 4)

    def test_uninstall_section_stops_the_runtime_before_deleting(self) -> None:
        lines = section(HARNESS_SCRIPT.read_text(encoding="utf-8"), "Uninstall")

        order = positions(
            lines,
            (
                "!insertmacro NSIS_HOOK_PREUNINSTALL",
                '!insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"',
                'Delete "$INSTDIR\\${MAINBINARYNAME}.exe"',
                'Delete "$INSTDIR\\${HARNESS_BRIDGE}.exe"',
                'Delete "$INSTDIR\\${HARNESS_MCP}.exe"',
                'Delete "$INSTDIR\\${HARNESS_CLI}.exe"',
            ),
        )
        self.assertEqual(order, sorted(order))

    def test_hooks_are_included_where_the_template_includes_them(self) -> None:
        lines = statements(HARNESS_SCRIPT.read_text(encoding="utf-8"))
        hooks = lines.index('!include "${HARNESS_HOOKS}"')

        # 模板先包含 utils.nsh，再包含钩子，最后才定义 INSTALLMODE 和 !addplugindir；钩子的顶层语句从这里起生效。
        self.assertLess(lines.index('!include "${HARNESS_TEMPLATE_DIR}\\utils.nsh"'), hooks)
        self.assertLess(hooks, lines.index('!define INSTALLMODE "currentUser"'))
        self.assertLess(hooks, lines.index('!addplugindir "${ADDITIONALPLUGINSPATH}"'))
        # AllowSkipFiles 只由钩子决定，精简安装器不能替它开或关。
        self.assertFalse([line for line in lines if line.startswith("AllowSkipFiles")])

    def test_every_harness_parameter_is_passed_to_makensis(self) -> None:
        defines = harness_defines(Path("hooks.nsh"), TOOLCHAIN, PRODUCT_IMAGES, Path("payload"), Path("out.exe"))

        used = set(re.findall(r"\$\{(HARNESS_[A-Z_]+)\}", HARNESS_SCRIPT.read_text(encoding="utf-8")))
        self.assertEqual(used, set(defines))
        self.assertEqual(
            set(re.findall(r"\$\{(HARNESS_[A-Z_]+)\}", PLACEHOLDER_SCRIPT.read_text(encoding="utf-8"))),
            {"HARNESS_OUTFILE"},
        )
        self.assertEqual(
            [defines[name] for name in ("HARNESS_DESKTOP", "HARNESS_BRIDGE", "HARNESS_MCP", "HARNESS_CLI")],
            ["agent-room-desktop", "agent-room-bridge", "agent-room-mcp", "agent-room"],
        )

    def test_mirrored_template_matches_the_desktop_build(self) -> None:
        package = json.loads((DESKTOP / "package.json").read_text(encoding="utf-8"))
        nsis = json.loads((DESKTOP / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))["bundle"]["windows"][
            "nsis"
        ]
        sidecars = json.loads((DESKTOP / "src-tauri" / "tauri.sidecar.conf.json").read_text(encoding="utf-8"))[
            "bundle"
        ]["externalBin"]
        crate = tomllib.loads((DESKTOP / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8"))

        # 升级 Tauri CLI 时，这里会提醒同步更新检查镜像的 NSIS、插件和模板。
        self.assertEqual(package["devDependencies"]["@tauri-apps/cli"], TAURI_CLI_VERSION)
        self.assertEqual(DESKTOP / "src-tauri" / nsis["installerHooks"], HOOKS)
        self.assertIn(f'!define INSTALLMODE "{nsis["installMode"]}"', HARNESS_SCRIPT.read_text(encoding="utf-8"))
        self.assertEqual(f"{crate['package']['name']}.exe", PRODUCT_IMAGES.desktop)
        self.assertEqual(
            [f"{Path(sidecar).name}.exe" for sidecar in sidecars],
            [PRODUCT_IMAGES.bridge, PRODUCT_IMAGES.mcp, PRODUCT_IMAGES.cli],
        )

    def test_hooks_stop_exactly_the_four_runtime_programs(self) -> None:
        source = HOOKS.read_text(encoding="utf-8")
        verify_hooks_cover_runtime(source)

        extra = source + "\n  nsExec::ExecToLog '\"$SYSDIR\\taskkill.exe\" /IM agent-room-updater.exe /T /F'\n"
        with self.assertRaisesRegex(InstallerHooksCheckFailure, "agent-room-updater.exe"):
            verify_hooks_cover_runtime(extra)
        with self.assertRaisesRegex(InstallerHooksCheckFailure, "agent-room-mcp.exe"):
            verify_hooks_cover_runtime(source.replace("agent-room-mcp.exe", "agent-room-bridge.exe"))


class ToolchainTests(unittest.TestCase):
    def test_every_download_is_pinned_to_an_https_url_and_hashes(self) -> None:
        for pinned in PINNED_FILES:
            with self.subTest(file=pinned.cache_name):
                self.assertTrue(pinned.url.startswith("https://"))
                self.assertRegex(pinned.sha256, r"^[0-9a-f]{64}$")
                if pinned.sha1 is not None:
                    self.assertRegex(pinned.sha1, r"^[0-9A-F]{40}$")
        self.assertEqual(len({pinned.cache_name for pinned in PINNED_FILES}), len(PINNED_FILES))
        # 模板文件按提交取，不按会被挪动的标签取。
        for template in (TEMPLATE_UTILS, TEMPLATE_ENGLISH):
            self.assertIn(f"/{TAURI_TEMPLATE_COMMIT}/", template.url)

    def test_both_pinned_hashes_must_match(self) -> None:
        data = b"nsis"
        sha256 = hashlib.sha256(data).hexdigest()
        sha1 = hashlib.sha1(data).hexdigest().upper()

        self.assertIsNone(pinned_mismatch(data, PinnedFile("a", "https://example.invalid/a", sha256, sha1)))
        self.assertIsNone(pinned_mismatch(data, PinnedFile("a", "https://example.invalid/a", sha256)))
        self.assertIn("SHA-256", pinned_mismatch(b"other", PinnedFile("a", "https://example.invalid/a", sha256, sha1)))
        self.assertIn("SHA-1", pinned_mismatch(data, PinnedFile("a", "https://example.invalid/a", sha256, "0" * 40)))

    def test_cached_file_is_reused_only_while_it_matches_the_pins(self) -> None:
        data = b"nsis"
        pinned = PinnedFile("tool.zip", "https://example.invalid/tool.zip", hashlib.sha256(data).hexdigest())
        fetch = MagicMock(return_value=data)

        with tempfile.TemporaryDirectory() as directory, contextlib.redirect_stdout(io.StringIO()):
            cache = Path(directory) / "cache"
            path = obtain(pinned, cache, fetch)
            self.assertEqual(path.read_bytes(), data)
            obtain(pinned, cache, fetch)
            fetch.assert_called_once_with(pinned.url)

            path.write_bytes(b"tampered")
            self.assertEqual(obtain(pinned, cache, fetch).read_bytes(), data)
            self.assertEqual(fetch.call_count, 2)

    def test_download_that_does_not_match_the_pins_is_rejected_and_not_cached(self) -> None:
        pinned = PinnedFile("tool.zip", "https://example.invalid/tool.zip", "0" * 64)

        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory) / "cache"
            with self.assertRaisesRegex(InstallerHooksCheckFailure, "SHA-256"):
                obtain(pinned, cache, lambda _url: b"other")
            self.assertFalse(cache.joinpath("tool.zip").exists())

    def test_nsis_archive_must_hold_only_the_expected_folder(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            good = root / "good.zip"
            with zipfile.ZipFile(good, "w") as bundle:
                bundle.writestr(f"{NSIS_ARCHIVE_ROOT}/makensis.exe", b"stub")
                bundle.writestr(f"{NSIS_ARCHIVE_ROOT}/Include/MUI2.nsh", b"; mui")
            nsis = extract_nsis(good, root / "good")
            self.assertEqual(nsis, root / "good" / NSIS_ARCHIVE_ROOT)
            self.assertTrue(nsis.joinpath("Include", "MUI2.nsh").is_file())

            stray = root / "stray.zip"
            with zipfile.ZipFile(stray, "w") as bundle:
                bundle.writestr(f"{NSIS_ARCHIVE_ROOT}/makensis.exe", b"stub")
                bundle.writestr("../outside.txt", b"escape")
            with self.assertRaisesRegex(InstallerHooksCheckFailure, "以外的内容"):
                extract_nsis(stray, root / "stray")
            self.assertFalse(root.joinpath("outside.txt").exists())

            empty = root / "empty.zip"
            with zipfile.ZipFile(empty, "w") as bundle:
                bundle.writestr(f"{NSIS_ARCHIVE_ROOT}/Include/MUI2.nsh", b"; mui")
            with self.assertRaisesRegex(InstallerHooksCheckFailure, "makensis.exe"):
                extract_nsis(empty, root / "empty")

    def test_every_placeholder_program_has_its_own_bytes(self) -> None:
        def compile_placeholder(_toolchain: Toolchain, _script: Path, defines: dict[str, str], _work: Path) -> None:
            Path(defines["HARNESS_OUTFILE"]).write_bytes(b"MZ placeholder")

        with tempfile.TemporaryDirectory() as directory, patch(
            "tools.windows_installer_hooks.compile_nsis", side_effect=compile_placeholder
        ) as compile_nsis:
            payloads = build_payloads(TOOLCHAIN, PRODUCT_IMAGES, Path(directory))

            self.assertEqual(compile_nsis.call_args.args[1], PLACEHOLDER_SCRIPT)
            for build in (payloads.previous, payloads.current):
                self.assertEqual(sorted(path.name for path in build.iterdir()), sorted(PRODUCT_IMAGES.names()))
                for path in build.iterdir():
                    self.assertTrue(path.read_bytes().startswith(b"MZ placeholder"))
            self.assertEqual(
                payloads.current_digests,
                {
                    name: hashlib.sha256(payloads.current.joinpath(name).read_bytes()).hexdigest()
                    for name in PRODUCT_IMAGES.names()
                },
            )
            # 八个文件各不相同，报告才分得清哪个程序是上一版、哪个是新版。
            self.assertEqual(sorted(payloads.labels.values()), ["上一版"] * 4 + ["新版"] * 4)

    def test_makensis_reads_utf8_like_tauri_and_fails_on_any_warning(self) -> None:
        command = makensis_command(TOOLCHAIN, Path("harness.nsi"), {"HARNESS_OUTFILE": "out.exe"})

        self.assertEqual(command[0], str(TOOLCHAIN.makensis))
        self.assertEqual(command[1:5], ["-INPUTCHARSET", "UTF8", "-OUTPUTCHARSET", "UTF8"])
        self.assertIn("-WX", command)
        # makensis 按顺序处理参数，-D 必须在脚本之前才生效。
        self.assertEqual(command[-2:], ["-DHARNESS_OUTFILE=out.exe", "harness.nsi"])


class IsolationTests(unittest.TestCase):
    def test_isolated_names_replace_every_runtime_program_and_nothing_else(self) -> None:
        source = HOOKS.read_text(encoding="utf-8")
        renamed = rename_runtime_images(source, ISOLATED_IMAGES)

        self.assertEqual(referenced_executables(renamed), frozenset(ISOLATED_IMAGES.names()))
        self.assertIn('"$SYSDIR\\taskkill.exe"', renamed)
        self.assertEqual(
            rename_runtime_images('/IM agent-room.exe; "AGENT-ROOM-MCP.EXE"; my-agent-room.exe', ISOLATED_IMAGES),
            '/IM arqa.exe; "arqa-mcp.exe"; my-agent-room.exe',
        )

    def test_real_program_names_only_run_on_github_hosted_runners(self) -> None:
        require_disposable_runner({"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"}, isolated=False)
        require_disposable_runner({}, isolated=True)
        for environment in ({}, {"GITHUB_ACTIONS": "true"}, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"}):
            with self.subTest(environment=environment):
                with self.assertRaisesRegex(InstallerHooksCheckFailure, "--isolated"):
                    require_disposable_runner(environment, isolated=False)


class ScenarioTests(unittest.TestCase):
    def test_scenarios_cover_install_and_uninstall_with_late_and_held_images(self) -> None:
        self.assertEqual(
            {(candidate.uninstall, candidate.release_after_seconds) for candidate in SCENARIOS},
            {(False, IMAGE_RELEASE_DELAY_SECONDS), (False, None), (True, IMAGE_RELEASE_DELAY_SECONDS), (True, None)},
        )
        timeout = re.search(r"!define AGENT_ROOM_STOP_TIMEOUT_MS (\d+)", HOOKS.read_text(encoding="utf-8"))
        assert timeout is not None
        # 晚放开的映像要在钩子等待上限之内；一直占着时，检查要等得到钩子超时中止。
        self.assertLess(IMAGE_RELEASE_DELAY_SECONDS * 1000 * 2, int(timeout.group(1)))
        self.assertGreater(INSTALLER_TIMEOUT_SECONDS * 1000, int(timeout.group(1)) * 2)

    def test_command_lines_keep_the_directory_last_and_unquoted(self) -> None:
        installer = Path("work") / "harness.exe"
        directory = Path("work") / "scenarios" / "install held"

        self.assertEqual(install_command(installer, directory), f'"{installer}" /S /D={directory}')
        self.assertEqual(uninstall_command(directory), f'"{directory / "uninstall.exe"}" /S _?={directory}')

    def test_late_release_install_must_replace_every_program(self) -> None:
        late = scenario("install-late-release")
        expected = expected_after(late, PREVIOUS, CURRENT, PRODUCT_IMAGES)
        installed = {**CURRENT, "uninstall.exe": "generated uninstaller"}

        self.assertEqual(directory_differences(expected, installed, LABELS), [])
        self.assertEqual(scenario_problems(late, InstallerRun(0, "", 4.4, True), [], []), [])

        # 旧钩子不等映像放开：桌面端写不进被跳过，其余三个换新，退出码照样是 0。
        mixed = {**installed, PRODUCT_IMAGES.desktop: PREVIOUS[PRODUCT_IMAGES.desktop]}
        differences = directory_differences(expected, mixed, LABELS)
        self.assertEqual(differences, ["agent-room-desktop.exe 是上一版，应为新版"])
        problems = scenario_problems(late, InstallerRun(0, "", 4.3, False), differences, [])
        self.assertEqual(len(problems), 2)
        self.assertIn("没有等到程序能写", problems[0])
        self.assertIn("agent-room-desktop.exe 是上一版", problems[1])

    def test_held_install_must_abort_and_leave_every_file_alone(self) -> None:
        held = scenario("install-held")
        expected = expected_after(held, PREVIOUS, CURRENT, PRODUCT_IMAGES)

        self.assertEqual(expected, PREVIOUS)
        aborted = InstallerRun(2, "Agent Room is still running or its files are in use", 22.5, None)
        self.assertEqual(scenario_problems(held, aborted, directory_differences(expected, PREVIOUS, LABELS), []), [])

        mixed = {**CURRENT, PRODUCT_IMAGES.desktop: PREVIOUS[PRODUCT_IMAGES.desktop], "uninstall.exe": "generated"}
        differences = directory_differences(expected, mixed, LABELS)
        self.assertEqual(differences[0], "多出 uninstall.exe")
        self.assertIn("agent-room-bridge.exe 是新版，应为上一版", differences)
        problems = "\n".join(scenario_problems(held, InstallerRun(0, "", 3.0, None), differences, ["agent-room-mcp.exe"]))
        self.assertIn("应中止并返回 2，实际退出码 0", problems)
        self.assertIn("标准输出", problems)
        self.assertIn("钩子没有结束这些占位程序：agent-room-mcp.exe", problems)

    def test_uninstall_removes_the_programs_but_keeps_the_running_uninstaller(self) -> None:
        installed = {**CURRENT, "uninstall.exe": "uninstaller"}

        late = expected_after(scenario("uninstall-late-release"), installed, CURRENT, PRODUCT_IMAGES)
        self.assertEqual(late, {"uninstall.exe": "uninstaller"})
        leftover = {**late, PRODUCT_IMAGES.desktop: CURRENT[PRODUCT_IMAGES.desktop]}
        self.assertEqual(directory_differences(late, leftover, LABELS), ["多出 agent-room-desktop.exe"])
        self.assertEqual(expected_after(scenario("uninstall-held"), installed, CURRENT, PRODUCT_IMAGES), installed)
        self.assertEqual(
            directory_differences(installed, late, LABELS),
            [f"缺少 {name}" for name in sorted(PRODUCT_IMAGES.names())],
        )


class ImageHoldTests(unittest.TestCase):
    def run_held(
        self,
        installer: MagicMock,
        release_after: float | None,
        *,
        wait_exit: MagicMock | None = None,
    ) -> tuple[InstallerRun, MagicMock, MagicMock]:
        steps = MagicMock()
        steps.popen.return_value = installer
        steps.hold.return_value.__enter__.return_value = steps.held
        if wait_exit is not None:
            steps.wait_exit = wait_exit
        desktop = MagicMock()
        with patch("tools.windows_installer_hooks.ExecutableImageHold", steps.hold), patch(
            "tools.windows_installer_hooks.subprocess.Popen", steps.popen
        ), patch("tools.windows_installer_hooks.wait_for_process_exit", steps.wait_exit), patch(
            "tools.windows_installer_hooks.time.sleep", steps.sleep
        ):
            run = run_while_desktop_image_is_held("setup.exe /S", Path("agent-room-desktop.exe"), desktop, release_after)
        return run, steps, desktop

    def test_image_is_released_only_after_the_desktop_is_gone(self) -> None:
        installer = MagicMock(returncode=0)
        installer.poll.side_effect = [None, 0]
        installer.communicate.return_value = (b"", None)

        run, steps, desktop = self.run_held(installer, IMAGE_RELEASE_DELAY_SECONDS)

        self.assertEqual(
            [entry for entry in steps.mock_calls if not entry[0].startswith(("hold().", "popen()."))],
            [
                call.hold(Path("agent-room-desktop.exe")),
                call.popen(
                    "setup.exe /S",
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                ),
                call.wait_exit(desktop, "桌面端占位程序"),
                call.sleep(IMAGE_RELEASE_DELAY_SECONDS),
                call.held.release(),
            ],
        )
        self.assertEqual((run.exit_code, run.running_when_released), (0, True))
        installer.kill.assert_not_called()

    def test_held_image_is_kept_until_the_installer_exits(self) -> None:
        installer = MagicMock(returncode=2)
        installer.poll.return_value = 2
        installer.communicate.return_value = (b"no files were changed\r\n", None)

        run, steps, _desktop = self.run_held(installer, None)

        steps.wait_exit.assert_not_called()
        steps.held.release.assert_not_called()
        self.assertEqual((run.exit_code, run.output, run.running_when_released), (2, "no files were changed", None))

    def test_installer_is_killed_when_the_desktop_never_exits(self) -> None:
        installer = MagicMock()
        installer.poll.return_value = None
        installer.communicate.return_value = (b"", None)
        wait_exit = MagicMock(side_effect=WindowsInstallerAcceptanceFailure("桌面端占位程序没有按安装器要求退出。"))

        with self.assertRaisesRegex(WindowsInstallerAcceptanceFailure, "没有按安装器要求退出"):
            self.run_held(installer, IMAGE_RELEASE_DELAY_SECONDS, wait_exit=wait_exit)
        installer.kill.assert_called_once_with()

    def test_installer_that_never_exits_is_killed_and_reported(self) -> None:
        installer = MagicMock()
        installer.poll.return_value = None
        installer.communicate.side_effect = [subprocess.TimeoutExpired("setup.exe", INSTALLER_TIMEOUT_SECONDS), (b"", None)]

        with self.assertRaisesRegex(InstallerHooksCheckFailure, f"{INSTALLER_TIMEOUT_SECONDS} 秒内没有退出"):
            self.run_held(installer, None)
        installer.kill.assert_called_once_with()


class WorkflowTests(unittest.TestCase):
    def test_workflow_runs_when_the_hooks_or_the_check_change(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        dependencies = [
            WORKFLOW,
            HOOKS,
            HARNESS_SCRIPT,
            PLACEHOLDER_SCRIPT,
            ROOT / "tools" / "windows_installer_hooks.py",
            ROOT / "tools" / "windows_installer_acceptance.py",
        ]

        for trigger in ("pull_request", "push"):
            paths = trigger_paths(workflow, trigger)
            for dependency in dependencies:
                relative = dependency.relative_to(ROOT).as_posix()
                with self.subTest(trigger=trigger, file=relative):
                    self.assertTrue(dependency.is_file())
                    self.assertTrue(
                        any(
                            relative == path or (path.endswith("/**") and relative.startswith(path[:-2]))
                            for path in paths
                        )
                    )

    def test_workflow_uses_real_program_names_on_one_standard_windows_runner(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        runners = [line.strip() for line in workflow.splitlines() if line.strip().startswith("runs-on:")]
        self.assertEqual(runners, ["runs-on: windows-latest"])
        self.assertIn("        run: python tools/windows_installer_hooks.py\n", workflow)
        self.assertNotIn("--isolated", workflow)


if __name__ == "__main__":
    unittest.main()
