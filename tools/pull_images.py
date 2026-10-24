#!/usr/bin/env python3
"""CI 起容器之前，先把 compose 文件里的镜像拉到本地，拉不下来隔一会儿再拉。

镜像站偶尔回 503 或限流（2026-10-24 Alpha 81 修复分支的集成作业就是 quay.io 回 503，
整个作业红了，只能重跑）。compose 自己拉镜像时出一次错就整个失败；它默认只拉本地没有的，
所以这里拉齐以后，后面的 `docker compose up` 不会再去镜像站。
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import re
import subprocess
import sys
import time
from typing import Callable, Final, Sequence


ROOT: Final = Path(__file__).resolve().parent.parent
# 第一次失败后依次等这么久再试，一共试 len(RETRY_DELAYS_SECONDS) + 1 次，最多多等 100 秒。
RETRY_DELAYS_SECONDS: Final = (10, 30, 60)
# 和 compose 一样几个镜像一起拉；太多了反而容易被限流。
PARALLEL_PULLS: Final = 4
SERVICE_LINE: Final = re.compile(r"^  ([A-Za-z0-9][A-Za-z0-9_.-]*):\s*$")
IMAGE_LINE: Final = re.compile(r"^    image:\s*(?P<quote>['\"]?)(?P<image>[^'\"\s]+)(?P=quote)\s*(?:#.*)?$")

Runner = Callable[[Sequence[str]], int]
Sleeper = Callable[[float], None]


class PullFailure(RuntimeError):
    """表示 compose 文件读不出镜像，或镜像重试完还是拉不下来。"""


def compose_images(path: Path, services: Sequence[str] = ()) -> list[str]:
    """按服务读出 `image:`，只认仓库里 compose 文件的两格缩进写法。"""

    by_service: dict[str, str] = {}
    in_services = False
    current: str | None = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            in_services = line.rstrip() == "services:"
            current = None
            continue
        if not in_services:
            continue
        service = SERVICE_LINE.match(line)
        if service:
            current = service.group(1)
            continue
        image = IMAGE_LINE.match(line)
        if image and current is not None:
            by_service[current] = image.group("image")

    missing = [name for name in services if name not in by_service]
    if missing:
        raise PullFailure(f"{path} 里没有这些服务的镜像：{', '.join(missing)}")
    chosen = [by_service[name] for name in services] if services else list(by_service.values())
    for image in chosen:
        if "$" in image:
            raise PullFailure(f"{path} 里的镜像带变量，没法预拉：{image}")
    return list(dict.fromkeys(chosen))


def run_docker(arguments: Sequence[str]) -> int:
    return subprocess.run(["docker", *arguments], cwd=ROOT, check=False).returncode


def run_docker_quietly(arguments: Sequence[str]) -> int:
    return subprocess.run(
        ["docker", *arguments],
        cwd=ROOT,
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode


def pull(
    image: str,
    *,
    run: Runner = run_docker,
    probe: Runner = run_docker_quietly,
    sleep: Sleeper = time.sleep,
) -> None:
    """本地有就不拉；拉失败按 RETRY_DELAYS_SECONDS 等一会儿再拉，都失败才报错。"""

    if probe(["image", "inspect", image]) == 0:
        print(f"{image}：本地已有", flush=True)
        return
    attempts = len(RETRY_DELAYS_SECONDS) + 1
    for attempt in range(1, attempts + 1):
        if run(["pull", "--quiet", image]) == 0:
            return
        if attempt == attempts:
            break
        delay = RETRY_DELAYS_SECONDS[attempt - 1]
        print(f"{image}：第 {attempt} 次没拉下来，{delay} 秒后再试", flush=True)
        sleep(delay)
    raise PullFailure(f"{image}：试了 {attempts} 次都没拉下来")


def pull_all(images: Sequence[str], **options: object) -> None:
    with ThreadPoolExecutor(max_workers=PARALLEL_PULLS) as executor:
        futures = [executor.submit(pull, image, **options) for image in images]
    failures = [str(error) for future in futures if (error := future.exception()) is not None]
    if failures:
        raise PullFailure("\n".join(failures))


def parse_arguments(arguments: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="预拉 compose 文件里的镜像，失败自动重试。")
    parser.add_argument("compose_file", type=Path, help="compose 文件，相对仓库根目录")
    parser.add_argument("services", nargs="*", help="只拉这些服务的镜像；不写就全拉")
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    options = parse_arguments(sys.argv[1:] if arguments is None else arguments)
    path = options.compose_file if options.compose_file.is_absolute() else ROOT / options.compose_file
    try:
        pull_all(compose_images(path, options.services))
    except PullFailure as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
