"""仓库里的 PowerShell 脚本在 Windows PowerShell 5.1 上也要跑得通。

Windows 上没装 pwsh 时，`tools/run-powershell.mjs` 退回 powershell.exe（5.1），justfile 在 Windows 上也用它。
CI 在 Linux 上用 pwsh 7，下面两处 5.1 才有的问题在那里看不出来，所以静态地卡住。
"""

from __future__ import annotations

import codecs
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# 2>$null、2>&1、*>… 这类重定向。前面要有空白，免得把正则字符串里的 [^>]*> 算进来。
STDERR_REDIRECT = re.compile(r"(?:^|\s)[2*]>")
RELAXED_PREFERENCE = re.compile(r"^\s*\$ErrorActionPreference\s*=\s*'(?:Silently)?Continue'", re.M)


def tracked_scripts() -> list[str]:
    completed = subprocess.run(
        ["git", "ls-files", "-z", "*.ps1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [name for name in completed.stdout.split("\0") if name]


def function_start(lines: list[str], index: int) -> int | None:
    """第 index 行所在函数的起始行。照仓库的写法，`function` 和收尾的 `}` 都顶格。"""
    for start in range(index, -1, -1):
        if lines[start].startswith("function "):
            return start
        if lines[start].startswith("}"):
            return None
    return None


def unguarded_stderr_redirects(script: str) -> list[int]:
    """重定向了 stderr、所在函数却没先放宽 $ErrorActionPreference 的行号；函数外面的一律算。"""
    lines = script.splitlines()
    offending = []
    for index, line in enumerate(lines):
        if not STDERR_REDIRECT.search(line.split("#", 1)[0]):
            continue
        start = function_start(lines, index)
        if start is None or not RELAXED_PREFERENCE.search("\n".join(lines[start:index])):
            offending.append(index + 1)
    return offending


class PowerShellScriptTests(unittest.TestCase):
    def test_每个脚本都存成带_bom_的_utf8(self) -> None:
        # 5.1 按系统代码页读没有 BOM 的脚本（中文 Windows 上是 GBK），中文字符串会解析失败。
        # 和 .editorconfig 里 *.ps1 的 charset = utf-8-bom 一致。
        scripts = tracked_scripts()
        self.assertIn("tools/device-grant-approve.ps1", scripts)
        missing = [
            name for name in scripts if not (ROOT / name).read_bytes().startswith(codecs.BOM_UTF8)
        ]
        self.assertEqual(missing, [])

    def test_重定向_stderr_之前都放宽了错误处理(self) -> None:
        # 5.1 在 $ErrorActionPreference = 'Stop' 下把重定向的原生命令 stderr 当成终止错误，
        # 本来只想看退出码的命令会让整个脚本停下；pwsh 7.2 起不会。
        scripts = tracked_scripts()
        self.assertIn("tools/dev-seed.ps1", scripts)
        offending = {
            name: lines
            for name in scripts
            if (lines := unguarded_stderr_redirects((ROOT / name).read_text(encoding="utf-8-sig")))
        }
        self.assertEqual(offending, {})

    def test_认得出在_stop_下直接重定向_stderr(self) -> None:
        stopped = (
            "$ErrorActionPreference = 'Stop'\n"
            "function Seed {\n"
            "  & docker s3api head-bucket 2>$null\n"
            "}\n"
            "& git rev-parse HEAD 2>&1 | Out-Null\n"
        )
        self.assertEqual(unguarded_stderr_redirects(stopped), [3, 5])

        relaxed = (
            "function Test-NativeCommand {\n"
            "  $ErrorActionPreference = 'Continue'\n"
            "  & $Command @Arguments 2>$null | Out-Null\n"
            "}\n"
        )
        self.assertEqual(unguarded_stderr_redirects(relaxed), [])

        not_redirects = (
            "$pattern = '(?s)<form[^>]+action=\"([^\"]+)\"[^>]*>.*?</form>'\n"
            "# 注释里写 2>$null 不算\n"
        )
        self.assertEqual(unguarded_stderr_redirects(not_redirects), [])


if __name__ == "__main__":
    unittest.main()
