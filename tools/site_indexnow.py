#!/usr/bin/env python3
"""公开发版以后，把 agentroom.chat 上改过的页报给 IndexNow（Bing、Yandex 等搜索引擎共用）。

网站跟着发版部署上线（specs/site-discovery/design.md），所以由 `site-indexnow.yml` 在“验证并发布
testing 签名发行”成功以后调用：读线上的 `sitemap.xml`，挑出 `lastmod` 不早于上一个公开版本发布那天的页，
带上密钥报给 api.indexnow.org。搜索引擎拿网站根目录的 `<密钥>.txt` 核对报信的是不是我们。

只是请搜索引擎早点来抓：报不出去不影响发版，工作流红了看一眼原因就行。

    python tools/site_indexnow.py                 # 报上一个公开版本以后改过的页
    python tools/site_indexnow.py --since 2026-11-19T06:45:45Z
    python tools/site_indexnow.py --all --dry-run # 只列出来，不报
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from typing import Callable, Final, Sequence
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
import xml.etree.ElementTree as ElementTree

ROOT: Final = Path(__file__).resolve().parent.parent
KEY_FILE: Final = ROOT / "apps" / "web" / "site" / "indexnow-key.txt"
HOST: Final = "agentroom.chat"
SITE: Final = f"https://{HOST}"
ENDPOINT: Final = "https://api.indexnow.org/indexnow"
KEY_PATTERN: Final = re.compile(r"^[A-Za-z0-9-]{8,128}$")
SITEMAP_NAMESPACE: Final = "{http://www.sitemaps.org/schemas/sitemap/0.9}"
# 200 收下了，202 收下了、密钥还在核对；403 是密钥不对，422 是网址不在这个域名下，429 是报得太勤。
ACCEPTED_STATUSES: Final = frozenset({200, 202})
FETCH_ATTEMPTS: Final = 3
USER_AGENT: Final = "agent-room-site-indexnow"


class IndexNowFailure(RuntimeError):
    """报不出去：网站、GitHub 或 IndexNow 没有给出能用的回答。"""


def read_key(path: Path = KEY_FILE) -> str:
    key = path.read_text(encoding="utf-8").strip()
    if not KEY_PATTERN.fullmatch(key):
        raise IndexNowFailure(f"{path} 里应当只有一个 IndexNow 密钥：8 到 128 个字母、数字或连字符")
    return key


def parse_sitemap(text: str) -> list[tuple[str, str | None]]:
    """站点地图里的每一页和它的 lastmod（没写就是 None），按出现的顺序。"""
    root = ElementTree.fromstring(text)
    entries: list[tuple[str, str | None]] = []
    for url in root.iter(f"{SITEMAP_NAMESPACE}url"):
        location = (url.findtext(f"{SITEMAP_NAMESPACE}loc") or "").strip()
        if not location:
            continue
        lastmod = (url.findtext(f"{SITEMAP_NAMESPACE}lastmod") or "").strip() or None
        entries.append((location, lastmod))
    return entries


def changed_urls(entries: Sequence[tuple[str, str | None]], since: str | None) -> list[str]:
    """`since` 那天及以后改过的页；没写 lastmod 的也报。`since` 为空时报全部。

    lastmod 只到日期（指南取 front matter 的 `updated`，首页取构建那天），所以按日期比：上一版发布
    当天改的页也算进来，多报一页不要紧，漏报才要紧。别的域名的网址 IndexNow 会整批拒绝，先去掉。
    """
    day = since[:10] if since else ""
    urls: list[str] = []
    for location, lastmod in entries:
        if not location.startswith(f"{SITE}/"):
            continue
        if not day or lastmod is None or lastmod[:10] >= day:
            urls.append(location)
    return urls


def payload(key: str, urls: Sequence[str]) -> dict[str, object]:
    return {"host": HOST, "key": key, "keyLocation": f"{SITE}/{key}.txt", "urlList": list(urls)}


def previous_release_time(
    repository: str,
    run: Callable[[Sequence[str]], str] | None = None,
) -> str | None:
    """上一个公开版本的发布时间：最新的公开版本是刚发的这一版，往前数一个。只有一版时为 None。"""
    runner = run or _gh
    output = runner(
        [
            "release", "list", "--repo", repository, "--exclude-drafts", "--exclude-pre-releases",
            "--limit", "2", "--json", "tagName,publishedAt",
        ]
    )
    releases = json.loads(output)
    if not isinstance(releases, list):
        raise IndexNowFailure("gh release list 没有回一个列表")
    times = [item.get("publishedAt") for item in releases if isinstance(item, dict)]
    if len(times) < 2:
        return None
    previous = times[1]
    if not isinstance(previous, str) or not previous:
        raise IndexNowFailure("上一个公开版本没有发布时间")
    return previous


def _gh(arguments: Sequence[str]) -> str:
    completed = subprocess.run(
        ["gh", *arguments], capture_output=True, text=True, encoding="utf-8", check=False
    )
    if completed.returncode != 0:
        raise IndexNowFailure(f"gh {' '.join(arguments[:2])} 失败：{completed.stderr.strip()}")
    return completed.stdout


def fetch_text(url: str) -> str:
    """GET 一个网址，失败隔 5、10 秒再试，三次都不行就报错。"""
    last_error: Exception | None = None
    for attempt in range(1, FETCH_ATTEMPTS + 1):
        request = Request(url, headers={"User-Agent": USER_AGENT, "Cache-Control": "no-cache"})
        try:
            with urlopen(request, timeout=30) as response:
                return response.read().decode("utf-8")
        except (HTTPError, URLError, TimeoutError) as error:
            last_error = error
        if attempt < FETCH_ATTEMPTS:
            time.sleep(attempt * 5)
    raise IndexNowFailure(f"{url} 打不开：{last_error}")


def submit(body: dict[str, object]) -> int:
    """报给 IndexNow，返回状态码；连不上时报错。"""
    request = Request(
        ENDPOINT,
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json; charset=utf-8", "User-Agent": USER_AGENT},
        method="POST",
    )
    try:
        with urlopen(request, timeout=30) as response:
            return int(response.status)
    except HTTPError as error:
        return int(error.code)
    except (URLError, TimeoutError) as error:
        raise IndexNowFailure(f"连不上 {ENDPOINT}：{error}") from error


def parse_args(arguments: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="把 agentroom.chat 上改过的页报给 IndexNow。")
    scope = parser.add_mutually_exclusive_group()
    scope.add_argument(
        "--since", help="只报这个时间（ISO 8601）那天及以后改过的页；不写就取上一个公开版本的发布时间"
    )
    scope.add_argument("--all", action="store_true", help="报站点地图里的全部页")
    parser.add_argument("--dry-run", action="store_true", help="只列出要报的页，不报")
    parser.add_argument("--repository", help="GitHub 仓库，默认取 GITHUB_REPOSITORY")
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_args(arguments)
    try:
        key = read_key()
        since: str | None = None
        if options.since:
            since = options.since
        elif not options.all:
            repository = (
                options.repository or os.environ.get("GITHUB_REPOSITORY") or "rainyflash/agent-room"
            )
            since = previous_release_time(repository)
        entries = parse_sitemap(fetch_text(f"{SITE}/sitemap.xml"))
        urls = changed_urls(entries, since)
        scope = f"{since} 那天及以后改过的" if since else "全部"
        print(f"站点地图 {len(entries)} 页，{scope}：{len(urls)} 页")
        if not urls:
            print("没有要报的页。")
            return 0
        if options.dry_run:
            print("\n".join(urls))
            return 0
        # IndexNow 先核对 keyLocation 上的密钥；文件还没上线就报，会被当成冒名拒掉。
        if fetch_text(f"{SITE}/{key}.txt").strip() != key:
            print(f"{SITE}/{key}.txt 上还不是这把密钥，先不报。", file=sys.stderr)
            return 1
        status = submit(payload(key, urls))
    except IndexNowFailure as error:
        print(error, file=sys.stderr)
        return 1
    print(f"IndexNow 回 {status}，报了 {len(urls)} 页")
    return 0 if status in ACCEPTED_STATUSES else 1


if __name__ == "__main__":
    raise SystemExit(main())
