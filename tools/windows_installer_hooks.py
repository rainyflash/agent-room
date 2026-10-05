#!/usr/bin/env python3
"""编译并实跑 Windows 安装器钩子 apps/desktop/src-tauri/windows/hooks.nsh。

钩子原本只在签名候选里随真实安装器一起编译和验收，写错了要等到发版才知道。这里用与 Tauri 2.11.1
相同的 NSIS 3.11 和 nsis_tauri_utils 插件，按模板的顺序编译一个精简安装器
（tools/nsis/installer_hooks_harness.nsi），有任何编译警告都算失败；装进去的是四个一直睡着的占位程序。
然后让占位程序从安装目录跑起来，由本进程按映像占住桌面端，检查：

- 桌面端被结束 3 秒后才放开：静默安装退出码 0、四个程序全部换新；静默卸载退出码 0、四个程序都删掉。
- 一直占着：静默安装和静默卸载都中止，退出码 2，安装目录一个文件都没动。

再模拟桌面端已经退出、它的 WebView 浏览器进程还开着本机数据（占着 lockfile），检查：

- 3 秒后放开：安装器等它放开才换文件，退出码 0、四个程序全部换新。
- 一直开着：中止，退出码 2，挪开的桌面端程序挪回原处，安装目录一个文件都没动。

钩子还在等的时候，再从外面看一眼：换文件期间的标记要被安装器独占（新版桌面端看到就退出）；
桌面端程序已挪开时，从安装目录启动它要失败（Agent 拉不起旧版）。装完、卸完、中止后都不许留下
挪开的程序和标记。

钩子会结束当前用户所有同名进程，所以用真实程序名只在 GitHub 托管的一次性 Runner 上跑。
本机检查加 --isolated：把钩子里的四个程序名换成 arqa-*，不碰已装的 Agent Room。
"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Mapping
from dataclasses import dataclass
import hashlib
import locale
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Final, Literal
import urllib.request
import zipfile

ROOT: Final = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools.windows_installer_acceptance import (
    BRIDGE_EXECUTABLE,
    CLI_EXECUTABLE,
    DESKTOP_EXECUTABLE,
    INSTALLER_ABORTED_EXIT_CODE,
    INSTALLER_MARKER,
    MCP_EXECUTABLE,
    ExecutableImageHold,
    WindowsInstallerAcceptanceFailure,
    desktop_relaunch_problem,
    marker_problem,
    sha256_file,
    wait_for_process_exit,
)

HOOKS: Final = ROOT / "apps" / "desktop" / "src-tauri" / "windows" / "hooks.nsh"
HARNESS_SCRIPT: Final = ROOT / "tools" / "nsis" / "installer_hooks_harness.nsi"
PLACEHOLDER_SCRIPT: Final = ROOT / "tools" / "nsis" / "runtime_placeholder.nsi"
DEFAULT_CACHE_DIR: Final = ROOT / ".local" / "windows-installer-hooks"
UNINSTALLER: Final = "uninstall.exe"

# 与 apps/desktop/package.json 的 @tauri-apps/cli 一致。升级 Tauri CLI 时，照新版 tauri-bundler 的
# crates/tauri-bundler/src/bundle/windows/nsis/ 更新下面的提交、地址和哈希，再对照新模板核对精简安装器。
TAURI_CLI_VERSION: Final = "2.11.1"
# tauri-cli-v2.11.1 标签指向的提交。按提交取模板文件，标签被挪动也不受影响。
TAURI_TEMPLATE_COMMIT: Final = "e5ae5b93cdd310045191cc0526f253140ad64b87"
TAURI_TEMPLATE_URL: Final = (
    "https://raw.githubusercontent.com/tauri-apps/tauri/"
    f"{TAURI_TEMPLATE_COMMIT}/crates/tauri-bundler/src/bundle/windows/nsis"
)


@dataclass(frozen=True, slots=True)
class PinnedFile:
    """固定地址和哈希的下载。sha1 与 tauri-bundler 校验用的相同，确认拿到的正是打包时用的那一份。"""

    cache_name: str
    url: str
    sha256: str
    sha1: str | None = None


NSIS_ARCHIVE: Final = PinnedFile(
    "nsis-3.11.zip",
    "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip",
    sha256="c7d27f780ddb6cffb4730138cd1591e841f4b7edb155856901cdf5f214394fa1",
    sha1="EF7FF767E5CBD9EDD22ADD3A32C9B8F4500BB10D",
)
TAURI_UTILS_PLUGIN: Final = PinnedFile(
    "nsis_tauri_utils-0.5.3.dll",
    "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/"
    "nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll",
    sha256="5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709",
    sha1="75197FEE3C6A814FE035788D1C34EAD39349B860",
)
TEMPLATE_UTILS: Final = PinnedFile(
    f"tauri-{TAURI_CLI_VERSION}-utils.nsh",
    f"{TAURI_TEMPLATE_URL}/utils.nsh",
    sha256="b27b407f886cca738e44e774f15e6b556b43ce52557a7c8891ee4eca7dd8013d",
)
TEMPLATE_ENGLISH: Final = PinnedFile(
    f"tauri-{TAURI_CLI_VERSION}-English.nsh",
    f"{TAURI_TEMPLATE_URL}/languages/English.nsh",
    sha256="1dad40b023707a61f828db1e184d9c1b029cb530c2dbbc4790db265872ef7b5e",
)
PINNED_FILES: Final = (NSIS_ARCHIVE, TAURI_UTILS_PLUGIN, TEMPLATE_UTILS, TEMPLATE_ENGLISH)
NSIS_ARCHIVE_ROOT: Final = "nsis-3.11"
MAX_DOWNLOAD_BYTES: Final = 16 * 1024 * 1024
DOWNLOAD_ATTEMPTS: Final = 3
MAKENSIS_TIMEOUT_SECONDS: Final = 300

# 旧钩子结束进程后只固定等 0.5 + 0.75 秒，模板再等 0.5 秒就写文件；映像晚 3 秒放开，那种写法就会漏掉桌面端。
IMAGE_RELEASE_DELAY_SECONDS: Final = 3.0
# 钩子最多等 20 秒，再留出结束进程和写文件的时间。
INSTALLER_TIMEOUT_SECONDS: Final = 120
PLACEHOLDER_STARTUP_SECONDS: Final = 0.5
PLACEHOLDER_EXIT_GRACE_SECONDS: Final = 5.0
# 预期目录里只要求存在、不比内容的文件（卸载器由 WriteUninstaller 现写）。
ANY_CONTENT: Final = "*"
# 钩子停 Agent Room 之前把桌面端程序挪到“程序名 + 这个后缀”（hooks.nsh 的 AGENT_ROOM_PARKED_DESKTOP）。
PARKED_SUFFIX: Final = ".old"
# 精简安装器的 BUNDLEID 前缀。每次检查再加一段随机后缀，钩子只会到这个目录里看 WebView 的本机数据。
HARNESS_BUNDLE_PREFIX: Final = "dev.agent-room.installer-hooks-check"

# 场景里占着的是桌面端程序的映像，还是 WebView 的本机数据。
DESKTOP_IMAGE: Final = "desktop-image"
WEBVIEW_DATA: Final = "webview-data"
HeldResource = Literal["desktop-image", "webview-data"]

# CreateFileW 的参数：WebView2 的浏览器进程就是这样开着本机数据目录里的 lockfile。
GENERIC_WRITE: Final = 0x4000_0000
FILE_SHARE_READ: Final = 0x0000_0001
CREATE_ALWAYS: Final = 2
FILE_ATTRIBUTE_NORMAL: Final = 0x0000_0080
FILE_FLAG_DELETE_ON_CLOSE: Final = 0x0400_0000


class InstallerHooksCheckFailure(WindowsInstallerAcceptanceFailure):
    """表示钩子没编译过、工具链对不上，或者运行中覆盖、卸载的结果与预期不符。"""


@dataclass(frozen=True, slots=True)
class RuntimeImages:
    """安装器要停掉并覆盖的四个程序，顺序同模板写文件的顺序：先主程序，再三个 sidecar。"""

    desktop: str
    bridge: str
    mcp: str
    cli: str

    def names(self) -> tuple[str, str, str, str]:
        return (self.desktop, self.bridge, self.mcp, self.cli)


PRODUCT_IMAGES: Final = RuntimeImages(DESKTOP_EXECUTABLE, BRIDGE_EXECUTABLE, MCP_EXECUTABLE, CLI_EXECUTABLE)
# 本机检查用的名字：钩子只会结束这些占位程序，碰不到已装的 Agent Room 和宿主拉起的 MCP。
ISOLATED_IMAGES: Final = RuntimeImages("arqa-desktop.exe", "arqa-bridge.exe", "arqa-mcp.exe", "arqa.exe")
# 钩子里出现的程序名。除了系统自带的 taskkill，都必须是上面四个之一。
EXECUTABLE_NAME: Final = re.compile(r"[A-Za-z0-9_.-]+\.exe(?![A-Za-z0-9_])", re.IGNORECASE)
SYSTEM_EXECUTABLES: Final = frozenset({"taskkill.exe"})


@dataclass(frozen=True, slots=True)
class Scenario:
    key: str
    label: str
    uninstall: bool
    # 桌面端被结束后，再占多少秒才放开；None 表示一直占到安装器退出。
    release_after_seconds: float | None
    held: HeldResource = DESKTOP_IMAGE


SCENARIOS: Final = (
    Scenario(
        "install-late-release",
        f"运行中覆盖安装，桌面端退出 {IMAGE_RELEASE_DELAY_SECONDS:g} 秒后才放开映像",
        False,
        IMAGE_RELEASE_DELAY_SECONDS,
    ),
    Scenario(
        "install-webview-late-release",
        f"覆盖安装，桌面端已退出、WebView 还开着本机数据 {IMAGE_RELEASE_DELAY_SECONDS:g} 秒",
        False,
        IMAGE_RELEASE_DELAY_SECONDS,
        WEBVIEW_DATA,
    ),
    Scenario("install-held", "运行中覆盖安装，映像一直被占着", False, None),
    Scenario("install-webview-held", "覆盖安装，桌面端已退出、WebView 一直开着本机数据", False, None, WEBVIEW_DATA),
    Scenario(
        "uninstall-late-release",
        f"运行中卸载，桌面端退出 {IMAGE_RELEASE_DELAY_SECONDS:g} 秒后才放开映像",
        True,
        IMAGE_RELEASE_DELAY_SECONDS,
    ),
    Scenario("uninstall-held", "运行中卸载，映像一直被占着", True, None),
)


@dataclass(frozen=True, slots=True)
class Toolchain:
    makensis: Path
    plugins: Path
    template: Path


@dataclass(frozen=True, slots=True)
class Payloads:
    """四个占位程序的“上一版”（预先放进安装目录）和“新版”（编进精简安装器）。"""

    previous: Path
    current: Path
    # 新版每个程序的 SHA-256。
    current_digests: dict[str, str]
    # SHA-256 到“上一版”“新版”，报告里说清楚装进去的是哪一版。
    labels: dict[str, str]


@dataclass(frozen=True, slots=True)
class InstallerRun:
    exit_code: int
    output: str
    seconds: float
    # 放开那一刻安装器是否还在等；一直占着时没有这一刻，为 None。
    running_when_released: bool | None
    # 安装器还在等的时候从外面看到的问题（桌面端还能启动、标记没被独占）。
    midway_problems: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class ScenarioResult:
    scenario: Scenario
    exit_code: int | None
    seconds: float
    problems: tuple[str, ...]
    output: str


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--hooks", type=Path, default=HOOKS, help="要检查的钩子文件，默认是仓库里的 hooks.nsh")
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=DEFAULT_CACHE_DIR,
        help="存放按哈希校验过的 NSIS、插件和模板文件，对得上就不再下载",
    )
    parser.add_argument(
        "--isolated",
        action="store_true",
        help="把四个程序名换成 arqa-*，本机检查时不碰已装的 Agent Room",
    )
    return parser.parse_args(argv)


def download(url: str) -> bytes:
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            with urllib.request.urlopen(url, timeout=60) as response:
                data: bytes = response.read(MAX_DOWNLOAD_BYTES + 1)
            break
        except OSError as error:
            if attempt == DOWNLOAD_ATTEMPTS:
                raise InstallerHooksCheckFailure(f"下载 {url} 失败：{error}") from error
            time.sleep(2 * attempt)
    if len(data) > MAX_DOWNLOAD_BYTES:
        raise InstallerHooksCheckFailure(f"{url} 超过 {MAX_DOWNLOAD_BYTES} 字节。")
    return data


def pinned_mismatch(data: bytes, pinned: PinnedFile) -> str | None:
    """内容与固定哈希对不上时说明哪一项不对，对得上时返回 None。"""

    actual = hashlib.sha256(data).hexdigest()
    if actual != pinned.sha256:
        return f"SHA-256 是 {actual}，应为 {pinned.sha256}"
    if pinned.sha1 is not None:
        actual = hashlib.sha1(data).hexdigest().upper()
        if actual != pinned.sha1:
            return f"SHA-1 是 {actual}，应为 {pinned.sha1}"
    return None


def obtain(pinned: PinnedFile, cache_dir: Path, fetch: Callable[[str], bytes] = download) -> Path:
    """返回按固定哈希校验过的文件：缓存里的对得上就直接用，否则重新下载、校验后存进缓存。"""

    cached = cache_dir / pinned.cache_name
    if cached.is_file() and pinned_mismatch(cached.read_bytes(), pinned) is None:
        print(f"复用缓存：{pinned.cache_name}", flush=True)
        return cached
    data = fetch(pinned.url)
    problem = pinned_mismatch(data, pinned)
    if problem is not None:
        raise InstallerHooksCheckFailure(f"{pinned.url} 与固定的哈希不符：{problem}。")
    cache_dir.mkdir(parents=True, exist_ok=True)
    partial = cached.with_name(f"{cached.name}.partial")
    partial.write_bytes(data)
    os.replace(partial, cached)
    print(f"已下载并校验：{pinned.cache_name}", flush=True)
    return cached


def extract_nsis(archive: Path, destination: Path) -> Path:
    """把 NSIS 解压到 destination，返回里面的 nsis-3.11 目录。压缩包已经按哈希校验过。"""

    with zipfile.ZipFile(archive) as bundle:
        stray = [name for name in bundle.namelist() if not name.startswith(f"{NSIS_ARCHIVE_ROOT}/")]
        if stray:
            raise InstallerHooksCheckFailure(f"NSIS 压缩包里有 {NSIS_ARCHIVE_ROOT}/ 以外的内容：{stray[0]}")
        # extractall 会去掉绝对路径和 ..，解出的文件落不到目标目录外面。
        bundle.extractall(destination)
    root = destination / NSIS_ARCHIVE_ROOT
    if not root.joinpath("makensis.exe").is_file():
        raise InstallerHooksCheckFailure("NSIS 压缩包里没有 makensis.exe。")
    return root


def prepare_toolchain(cache_dir: Path, work_dir: Path, fetch: Callable[[str], bytes] = download) -> Toolchain:
    archive = obtain(NSIS_ARCHIVE, cache_dir, fetch)
    plugin = obtain(TAURI_UTILS_PLUGIN, cache_dir, fetch)
    utils = obtain(TEMPLATE_UTILS, cache_dir, fetch)
    english = obtain(TEMPLATE_ENGLISH, cache_dir, fetch)
    nsis = extract_nsis(archive, work_dir)
    # 和 tauri-bundler 一样放进 Plugins/x86-unicode/additional，模板用 !addplugindir 指向这里。
    plugins = nsis / "Plugins" / "x86-unicode" / "additional"
    plugins.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(plugin, plugins / "nsis_tauri_utils.dll")
    template = work_dir / "tauri-template"
    template.mkdir()
    shutil.copyfile(utils, template / "utils.nsh")
    shutil.copyfile(english, template / "English.nsh")
    return Toolchain(nsis / "makensis.exe", plugins, template)


def makensis_command(toolchain: Toolchain, script: Path, defines: Mapping[str, str]) -> list[str]:
    # 和 tauri-bundler 一样按 UTF-8 读写脚本；-WX 把任何警告都当作错误。
    return [
        str(toolchain.makensis),
        "-INPUTCHARSET",
        "UTF8",
        "-OUTPUTCHARSET",
        "UTF8",
        "-V2",
        "-WX",
        *(f"-D{name}={value}" for name, value in defines.items()),
        str(script),
    ]


def compile_nsis(toolchain: Toolchain, script: Path, defines: Mapping[str, str], work_dir: Path) -> None:
    # tauri-bundler 运行 makensis 前也去掉这两个变量，免得读到别处的 NSIS 配置。
    environment = {
        name: value for name, value in os.environ.items() if name.upper() not in {"NSISDIR", "NSISCONFDIR"}
    }
    completed = subprocess.run(
        makensis_command(toolchain, script, defines),
        cwd=work_dir,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=MAKENSIS_TIMEOUT_SECONDS,
    )
    if completed.returncode != 0:
        output = "\n".join(part.strip() for part in (completed.stdout, completed.stderr) if part.strip())
        raise InstallerHooksCheckFailure(
            f"makensis 编译 {script.name} 失败（退出码 {completed.returncode}）：\n{output}"
        )


def payload_marker(build: str, image: str) -> bytes:
    # PE 加载器不读文件末尾的附加数据，占位程序照常运行，每个文件的字节又各不相同。
    return f"\0agent-room installer hooks check: {build} {image}\0".encode()


def build_payloads(toolchain: Toolchain, images: RuntimeImages, work_dir: Path) -> Payloads:
    placeholder = work_dir / "runtime-placeholder.exe"
    compile_nsis(toolchain, PLACEHOLDER_SCRIPT, {"HARNESS_OUTFILE": str(placeholder)}, work_dir)
    program = placeholder.read_bytes()
    digests: dict[str, dict[str, str]] = {"previous": {}, "current": {}}
    for build, builds in digests.items():
        directory = work_dir / build
        directory.mkdir()
        for image in images.names():
            content = program + payload_marker(build, image)
            directory.joinpath(image).write_bytes(content)
            builds[image] = hashlib.sha256(content).hexdigest()
    labels = {
        **{digest: "上一版" for digest in digests["previous"].values()},
        **{digest: "新版" for digest in digests["current"].values()},
    }
    return Payloads(work_dir / "previous", work_dir / "current", digests["current"], labels)


def harness_defines(
    hooks: Path,
    toolchain: Toolchain,
    images: RuntimeImages,
    payload_dir: Path,
    installer: Path,
    bundle_id: str,
) -> dict[str, str]:
    return {
        "HARNESS_HOOKS": str(hooks),
        "HARNESS_TEMPLATE_DIR": str(toolchain.template),
        "HARNESS_PLUGINS_DIR": str(toolchain.plugins),
        "HARNESS_PAYLOAD_DIR": str(payload_dir),
        "HARNESS_DESKTOP": Path(images.desktop).stem,
        "HARNESS_BRIDGE": Path(images.bridge).stem,
        "HARNESS_MCP": Path(images.mcp).stem,
        "HARNESS_CLI": Path(images.cli).stem,
        "HARNESS_BUNDLEID": bundle_id,
        "HARNESS_OUTFILE": str(installer),
    }


def webview_lockfile(local_app_data: Path, bundle_id: str) -> Path:
    """WebView2 的浏览器进程开着本机数据时一直占着的文件，钩子按同样的路径去查。"""

    return local_app_data / bundle_id / "EBWebView" / "lockfile"


def referenced_executables(source: str) -> frozenset[str]:
    return frozenset(name.lower() for name in EXECUTABLE_NAME.findall(source)) - SYSTEM_EXECUTABLES


def verify_hooks_cover_runtime(source: str) -> None:
    """钩子处理的程序必须正好是这四个：多一个，本机隔离时换不掉它的名字；少一个，就有程序没停就被覆盖。"""

    referenced = referenced_executables(source)
    expected = frozenset(name.lower() for name in PRODUCT_IMAGES.names())
    unknown = sorted(referenced - expected)
    if unknown:
        raise InstallerHooksCheckFailure(
            f"钩子里有检查不认识的程序：{'、'.join(unknown)}。先把它加进 tools/windows_installer_hooks.py 的占位程序。"
        )
    missing = sorted(expected - referenced)
    if missing:
        raise InstallerHooksCheckFailure(f"钩子没有处理这些程序：{'、'.join(missing)}。")


def rename_runtime_images(source: str, images: RuntimeImages) -> str:
    """把钩子里的四个真实程序名换成 images 里对应的名字。"""

    renamed = {real.lower(): new for real, new in zip(PRODUCT_IMAGES.names(), images.names())}
    return EXECUTABLE_NAME.sub(lambda match: renamed.get(match.group(0).lower(), match.group(0)), source)


def require_disposable_runner(environment: Mapping[str, str], isolated: bool) -> None:
    if isolated:
        return
    if environment.get("GITHUB_ACTIONS") == "true" and environment.get("RUNNER_ENVIRONMENT") == "github-hosted":
        return
    raise InstallerHooksCheckFailure(
        "钩子会结束当前用户所有同名进程（桌面端、Bridge、MCP、CLI），用真实程序名只在 GitHub 托管的一次性 Runner 上跑。"
        "本机检查请加 --isolated，把程序名换成 arqa-*。"
    )


def install_command(installer: Path, directory: Path) -> str:
    # NSIS 要求 /D= 放在最后，而且不能加引号，路径里有空格也不加。
    return f'"{installer}" /S /D={directory}'


def uninstall_command(directory: Path) -> str:
    # 带 _?= 时卸载器在原地运行并返回真实退出码；不带它，卸载器会复制到临时目录再启动，立即返回 0。
    return f'"{directory / UNINSTALLER}" /S _?={directory}'


def snapshot(directory: Path) -> dict[str, str]:
    return {
        path.relative_to(directory).as_posix(): sha256_file(path)
        for path in sorted(directory.rglob("*"))
        if path.is_file()
    }


def expected_after(
    scenario: Scenario,
    before: Mapping[str, str],
    current: Mapping[str, str],
    images: RuntimeImages,
) -> dict[str, str]:
    """场景结束后安装目录应有的样子：文件名到 SHA-256，ANY_CONTENT 表示只要求存在。"""

    if scenario.release_after_seconds is None:
        return dict(before)
    if scenario.uninstall:
        # 卸载器带着 _?= 在原地运行，删不掉自己，安装目录也就留着。
        return {name: digest for name, digest in before.items() if name not in images.names()}
    return {**before, **{image: current[image] for image in images.names()}, UNINSTALLER: ANY_CONTENT}


def directory_differences(
    expected: Mapping[str, str],
    actual: Mapping[str, str],
    labels: Mapping[str, str],
) -> list[str]:
    differences = [f"缺少 {name}" for name in sorted(expected.keys() - actual.keys())]
    differences += [f"多出 {name}" for name in sorted(actual.keys() - expected.keys())]
    for name in sorted(expected.keys() & actual.keys()):
        if expected[name] not in (ANY_CONTENT, actual[name]):
            differences.append(
                f"{name} 是{labels.get(actual[name], '别的内容')}，应为{labels.get(expected[name], '别的内容')}"
            )
    return differences


def scenario_problems(
    scenario: Scenario,
    run: InstallerRun,
    differences: list[str],
    survivors: list[str],
) -> list[str]:
    problems: list[str] = list(run.midway_problems)
    held = "WebView 的本机数据" if scenario.held == WEBVIEW_DATA else "映像"
    if scenario.release_after_seconds is None:
        if run.exit_code != INSTALLER_ABORTED_EXIT_CODE:
            problems.append(
                f"{held}一直被占着，应中止并返回 {INSTALLER_ABORTED_EXIT_CODE}，实际退出码 {run.exit_code}。"
            )
        if not run.output:
            problems.append("静默中止时没有把原因写到标准输出。")
    else:
        if run.running_when_released is False:
            problems.append(f"{held}还没放开安装器就结束了，没有等它放开再动文件。")
        if run.exit_code != 0:
            problems.append(f"{held}放开后应顺利完成（退出码 0），实际退出码 {run.exit_code}。")
    if differences:
        problems.append(f"安装目录与预期不符：{'；'.join(differences)}。")
    if survivors:
        problems.append(f"钩子没有结束这些占位程序：{'、'.join(survivors)}。")
    return problems


def decode_output(output: bytes) -> str:
    # 钩子用 FileWrite 按系统 ANSI 代码页写。中文在英文代码页的 Runner 上本来就写不出来，英文那半句总能读到。
    return output.decode(locale.getencoding(), errors="replace").strip()


def running_images(scenario: Scenario, images: RuntimeImages) -> tuple[str, ...]:
    """场景开始时在运行的占位程序。

    WebView 还开着本机数据的场景里，桌面端已经退出（WebView 的浏览器进程还没退干净），只剩三个 sidecar。
    这时钩子能把桌面端程序挪开，检查也就看得到 Agent 拉不起它。桌面端占位程序在运行时挪不开：
    NSIS 编出来的程序一直以不许删改的方式开着自己，真正的桌面端没有这个问题。
    """

    if scenario.held == WEBVIEW_DATA:
        return (images.bridge, images.mcp, images.cli)
    return images.names()


def start_placeholders(directory: Path, names: tuple[str, ...]) -> list[subprocess.Popen[bytes]]:
    processes: list[subprocess.Popen[bytes]] = []
    try:
        for image in names:
            processes.append(
                subprocess.Popen(
                    (str(directory / image),),
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            )
        time.sleep(PLACEHOLDER_STARTUP_SECONDS)
        exited = [image for image, process in zip(names, processes) if process.poll() is not None]
        if exited:
            raise InstallerHooksCheckFailure(f"占位程序没能保持运行：{'、'.join(exited)}。")
    except BaseException:
        stop_placeholders(processes)
        raise
    return processes


def stop_placeholders(processes: list[subprocess.Popen[bytes]]) -> None:
    for process in processes:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=30)


def surviving_placeholders(processes: list[subprocess.Popen[bytes]], names: tuple[str, ...]) -> list[str]:
    """安装器退出后还在运行的占位程序。钩子应当已经把它们都结束了。"""

    deadline = time.monotonic() + PLACEHOLDER_EXIT_GRACE_SECONDS
    for process in processes:
        try:
            process.wait(timeout=max(0.0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            pass
    return [image for image, process in zip(names, processes) if process.poll() is None]


class WebViewDataHold:
    """像 WebView2 的浏览器进程那样开着本机数据目录里的 lockfile：写、只共享读、关掉就删。

    浏览器进程开着本机数据时一直这样占着它。钩子看到它被占着，就要等到放开再换文件，不然新版
    桌面端一启动就和还没退干净的浏览器进程抢同一份数据。
    """

    def __init__(self, path: Path) -> None:
        import ctypes
        from ctypes import wintypes

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        create_file = kernel32.CreateFileW
        create_file.argtypes = (
            wintypes.LPCWSTR,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.LPVOID,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.HANDLE,
        )
        create_file.restype = wintypes.HANDLE
        self._close_handle = kernel32.CloseHandle
        self._close_handle.argtypes = (wintypes.HANDLE,)
        self._close_handle.restype = wintypes.BOOL
        path.parent.mkdir(parents=True, exist_ok=True)
        handle = create_file(
            str(path),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_DELETE_ON_CLOSE,
            None,
        )
        if handle is None or handle == ctypes.c_void_p(-1).value:
            raise InstallerHooksCheckFailure(f"无法占用 {path}（错误 {ctypes.get_last_error()}）。")
        self._handle: int | None = handle

    def release(self) -> None:
        if self._handle is not None:
            self._close_handle(self._handle)
            self._handle = None

    def __enter__(self) -> WebViewDataHold:
        return self

    def __exit__(self, *_: object) -> None:
        self.release()


def midway_problems(directory: Path, images: RuntimeImages, *, desktop_parked: bool) -> list[str]:
    """钩子还在等 Agent Room 退干净时，从外面看到的问题。

    desktop_parked 为真时，桌面端程序应已被挪开：从安装目录启动它必须失败。
    """

    found = [marker_problem(directory / INSTALLER_MARKER)]
    if desktop_parked:
        found.append(desktop_relaunch_problem(directory / images.desktop))
    return [problem for problem in found if problem is not None]


def run_while_held(
    command: str,
    hold: ExecutableImageHold | WebViewDataHold,
    stopped: tuple[subprocess.Popen[bytes], str],
    release_after_seconds: float | None,
    midway: Callable[[], list[str]] = list,
) -> InstallerRun:
    """占着桌面端映像或 WebView 的本机数据运行安装器或卸载器。

    release_after_seconds 为 None 时一直占到它退出；否则等钩子结束 stopped 这个占位程序，再占这么多秒才放开，
    并记下放开那一刻它是否还在等。还在等的话，放开之前先用 midway 从外面看一眼。
    """

    process, label = stopped
    with hold:
        started = time.monotonic()
        installer = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            running_when_released: bool | None = None
            seen_midway: list[str] = []
            if release_after_seconds is not None:
                wait_for_process_exit(process, label)
                time.sleep(release_after_seconds)
                running_when_released = installer.poll() is None
                if running_when_released:
                    seen_midway = midway()
                hold.release()
            output, _ = installer.communicate(timeout=INSTALLER_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired as error:
            raise InstallerHooksCheckFailure(f"安装器 {INSTALLER_TIMEOUT_SECONDS} 秒内没有退出。") from error
        finally:
            if installer.poll() is None:
                installer.kill()
                installer.communicate()
        seconds = time.monotonic() - started
    return InstallerRun(
        installer.returncode, decode_output(output), seconds, running_when_released, tuple(seen_midway)
    )


def install_placeholders_cleanly(
    installer: Path,
    directory: Path,
    expected: Mapping[str, str],
    labels: Mapping[str, str],
) -> None:
    """卸载场景先要有一次干净安装：没有进程在跑时，安装器应直接装好四个新版程序和卸载器。"""

    completed = subprocess.run(
        install_command(installer, directory),
        check=False,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=INSTALLER_TIMEOUT_SECONDS,
    )
    differences = directory_differences(expected, snapshot(directory), labels)
    if completed.returncode != 0 or differences:
        detail = "；".join(differences) or decode_output(completed.stdout) or "无输出"
        raise InstallerHooksCheckFailure(
            f"没有进程在跑时的静默安装就不对（退出码 {completed.returncode}）：{detail}"
        )


def run_scenario(
    scenario: Scenario,
    installer: Path,
    images: RuntimeImages,
    payloads: Payloads,
    root: Path,
    webview_lock: Path,
) -> ScenarioResult:
    directory = root / scenario.key
    current = payloads.current_digests
    started = time.monotonic()
    try:
        if scenario.uninstall:
            install_placeholders_cleanly(installer, directory, {**current, UNINSTALLER: ANY_CONTENT}, payloads.labels)
            command = uninstall_command(directory)
        else:
            shutil.copytree(payloads.previous, directory)
            command = install_command(installer, directory)
        before = snapshot(directory)
        names = running_images(scenario, images)
        processes = start_placeholders(directory, names)
        try:
            webview = scenario.held == WEBVIEW_DATA
            hold = WebViewDataHold(webview_lock) if webview else ExecutableImageHold(directory / images.desktop)
            run = run_while_held(
                command,
                hold,
                (processes[0], f"{names[0]} 占位程序"),
                scenario.release_after_seconds,
                lambda: midway_problems(directory, images, desktop_parked=webview),
            )
            survivors = surviving_placeholders(processes, names)
        finally:
            stop_placeholders(processes)
        differences = directory_differences(
            expected_after(scenario, before, current, images),
            snapshot(directory),
            payloads.labels,
        )
        problems = scenario_problems(scenario, run, differences, survivors)
        return ScenarioResult(scenario, run.exit_code, run.seconds, tuple(problems), run.output)
    except (OSError, subprocess.SubprocessError, WindowsInstallerAcceptanceFailure) as error:
        return ScenarioResult(scenario, None, time.monotonic() - started, (str(error),), "")


def print_result(result: ScenarioResult) -> None:
    exit_code = "无" if result.exit_code is None else str(result.exit_code)
    verdict = "失败" if result.problems else "通过"
    print(
        f"{verdict}：{result.scenario.label}（退出码 {exit_code}，用时 {result.seconds:.1f} 秒）",
        flush=True,
    )
    for problem in result.problems:
        print(f"  - {problem}", flush=True)
    if result.problems and result.output:
        print(f"  安装器输出：{result.output}", flush=True)


def check(hooks: Path, cache_dir: Path, isolated: bool) -> list[ScenarioResult]:
    if os.name != "nt":
        raise InstallerHooksCheckFailure("安装器钩子检查只能在 Windows 上运行。")
    require_disposable_runner(os.environ, isolated)
    source = hooks.read_text(encoding="utf-8")
    verify_hooks_cover_runtime(source)
    images = ISOLATED_IMAGES if isolated else PRODUCT_IMAGES
    print(f"检查 {hooks}，程序名用{'隔离的 arqa-*' if isolated else '真实的 agent-room-*'}。", flush=True)
    # 钩子到 %LOCALAPPDATA%\<BUNDLEID>\EBWebView 下看 WebView 的本机数据。检查用一个只属于这次的
    # BUNDLEID，碰不到已装的 Agent Room 的数据，跑完连目录一起删掉。
    bundle_id = f"{HARNESS_BUNDLE_PREFIX}-{secrets.token_hex(4)}"
    bundle_data = Path(os.environ["LOCALAPPDATA"]) / bundle_id
    with tempfile.TemporaryDirectory(prefix="agent-room-installer-hooks-", ignore_cleanup_errors=True) as temporary:
        try:
            work_dir = Path(temporary)
            toolchain = prepare_toolchain(cache_dir, work_dir)
            if isolated:
                hooks = work_dir / "hooks.nsh"
                hooks.write_text(rename_runtime_images(source, images), encoding="utf-8")
            payloads = build_payloads(toolchain, images, work_dir)
            installer = work_dir / "installer-hooks-harness.exe"
            compile_nsis(
                toolchain,
                HARNESS_SCRIPT,
                harness_defines(hooks, toolchain, images, payloads.current, installer, bundle_id),
                work_dir,
            )
            print("编译通过：钩子和精简安装器没有任何警告。", flush=True)
            scenarios = work_dir / "scenarios"
            scenarios.mkdir()
            results: list[ScenarioResult] = []
            webview_lock = webview_lockfile(bundle_data.parent, bundle_id)
            for scenario in SCENARIOS:
                results.append(run_scenario(scenario, installer, images, payloads, scenarios, webview_lock))
                print_result(results[-1])
        finally:
            shutil.rmtree(bundle_data, ignore_errors=True)
    return results


def main(argv: list[str] | None = None) -> int:
    arguments = parse_args(argv)
    try:
        results = check(arguments.hooks.resolve(), arguments.cache_dir.resolve(), arguments.isolated)
    except (OSError, subprocess.SubprocessError, WindowsInstallerAcceptanceFailure) as error:
        print(f"Windows 安装器钩子检查失败：{error}", file=sys.stderr)
        return 1
    failed = sum(1 for result in results if result.problems)
    if failed:
        print(f"Windows 安装器钩子检查失败：{failed} 个场景与预期不符。", file=sys.stderr)
        return 1
    print("Windows 安装器钩子检查通过。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
