#!/usr/bin/env python3
"""在真实 Synapse 上执行 250 人大厅、状态续租和消息速率容量场景。"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import UTC, datetime
import hashlib
import json
from pathlib import Path
import sys
import time
from typing import Callable, Final, Iterable, TypeAlias, TypeVar
from urllib.parse import urlencode
import uuid

if __package__:
    from .capacity import (
        CapacityFailure,
        git_revision,
        percentile,
        require_git_revision,
        write_json,
    )
    from .federation import (
        ALPHA,
        FederationFailure,
        MatrixUser,
        down,
        encoded,
        matrix_request,
        prepare,
        read_environment,
        register_user,
        up,
    )
else:
    from capacity import (
        CapacityFailure,
        git_revision,
        percentile,
        require_git_revision,
        write_json,
    )
    from federation import (
        ALPHA,
        FederationFailure,
        MatrixUser,
        down,
        encoded,
        matrix_request,
        prepare,
        read_environment,
        register_user,
        up,
    )


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT: Final = ROOT / "artifacts" / "capacity" / "matrix-report.json"
MEMBER_COUNT: Final = 250
SUSTAINED_RATE: Final = 10
BURST_COUNT: Final = 50
RATE_LIMIT_ATTEMPTS: Final = 24
RATE_LIMIT_JITTER_SECONDS: Final = 0.4
REGISTRATION_WORKERS: Final = 24
JOIN_WORKERS: Final = 8
STATE_WORKERS: Final = 24
T = TypeVar("T")
CapacityMessageSender: TypeAlias = Callable[[MatrixUser, str, str], tuple[str, float]]


class MatrixCapacityFailure(RuntimeError):
    """表示真实 Matrix 容量目标没有达到。"""


def parallel_map(
    function: Callable[[int], T], values: Iterable[int], workers: int
) -> list[T]:
    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = [executor.submit(function, value) for value in values]
        return [future.result() for future in as_completed(futures)]


def create_public_room(owner: MatrixUser) -> str:
    response = matrix_request(
        owner.peer,
        "POST",
        "/_matrix/client/v3/createRoom",
        token=owner.access_token,
        payload={
            "name": "Agent Room task 39 capacity lobby",
            "topic": "Real 250 member capacity evidence",
            "visibility": "public",
            "preset": "public_chat",
            "room_alias_name": f"capacity-{uuid.uuid4().hex[:12]}",
            "creation_content": {"m.federate": True},
        },
    )
    room_id = response.get("room_id")
    if not isinstance(room_id, str):
        raise MatrixCapacityFailure("容量房间创建响应缺少 room_id。")
    return room_id


def join_room(user: MatrixUser, room_id: str) -> float:
    started = time.perf_counter()
    last_response: dict[str, object] = {}
    for attempt in range(RATE_LIMIT_ATTEMPTS):
        response = matrix_request(
            user.peer,
            "POST",
            f"/_matrix/client/v3/join/{encoded(room_id)}",
            token=user.access_token,
            payload={},
            expected_statuses=(200, 429),
        )
        if response.get("_status") == 200:
            break
        last_response = response
        time.sleep(retry_delay(response, attempt, user.user_id))
    else:
        raise rate_limit_exhausted("加入大厅", user.user_id, last_response)
    return (time.perf_counter() - started) * 1_000.0


def joined_member_count(owner: MatrixUser, room_id: str) -> int:
    response = matrix_request(
        owner.peer,
        "GET",
        f"/_matrix/client/v3/rooms/{encoded(room_id)}/joined_members",
        token=owner.access_token,
    )
    joined = response.get("joined")
    if not isinstance(joined, dict):
        raise MatrixCapacityFailure("joined_members 响应缺少成员映射。")
    return len(joined)


def renew_state(owner: MatrixUser, room_id: str, ordinal: int) -> float:
    started = time.perf_counter()
    discriminator = f"state:{ordinal}"
    last_response: dict[str, object] = {}
    for attempt in range(RATE_LIMIT_ATTEMPTS):
        response = matrix_request(
            owner.peer,
            "PUT",
            (
                f"/_matrix/client/v3/rooms/{encoded(room_id)}/state/"
                "io.github.rainyflash.agentroom.agent.status.v1/"
                f"capacity-agent-{ordinal:03d}"
            ),
            token=owner.access_token,
            payload={
                "schemaVersion": "1.0",
                "state": "working" if ordinal % 3 else "idle",
                "leaseSeconds": 90,
                "sequence": 1,
            },
            expected_statuses=(200, 429),
        )
        if response.get("_status") == 200:
            break
        last_response = response
        time.sleep(retry_delay(response, attempt, discriminator))
    else:
        raise rate_limit_exhausted("状态续租", discriminator, last_response)
    return (time.perf_counter() - started) * 1_000.0


def send_capacity_message(owner: MatrixUser, room_id: str, label: str) -> tuple[str, float]:
    started = time.perf_counter()
    transaction_id = f"capacity-{uuid.uuid4().hex}"
    event_id: str | None = None
    last_response: dict[str, object] = {}
    for attempt in range(RATE_LIMIT_ATTEMPTS):
        response = matrix_request(
            owner.peer,
            "PUT",
            (
                f"/_matrix/client/v3/rooms/{encoded(room_id)}/send/"
                "io.github.rainyflash.agentroom.message.preview.v1/"
                f"{encoded(transaction_id)}"
            ),
            token=owner.access_token,
            payload={"schemaVersion": "1.0", "body": label},
            expected_statuses=(200, 429),
        )
        if response.get("_status") == 200:
            candidate = response.get("event_id")
            if not isinstance(candidate, str):
                raise MatrixCapacityFailure("消息接受响应缺少 event_id。")
            event_id = candidate
            break
        last_response = response
        time.sleep(retry_delay(response, attempt, transaction_id))
    if event_id is None:
        raise rate_limit_exhausted("消息发送", transaction_id, last_response)
    return event_id, (time.perf_counter() - started) * 1_000.0


def retry_delay(
    response: dict[str, object], attempt: int, discriminator: str = ""
) -> float:
    retry_after_ms = response.get("retry_after_ms")
    if isinstance(retry_after_ms, int) and retry_after_ms > 0:
        base_delay = min(30.0, retry_after_ms / 1_000.0)
    else:
        base_delay = min(5.0, 0.1 * (2**attempt))
    digest = hashlib.sha256(f"{discriminator}:{attempt}".encode("utf-8")).digest()
    jitter = int.from_bytes(digest[:2], "big") / 65_535
    return base_delay + jitter * RATE_LIMIT_JITTER_SECONDS


def rate_limit_exhausted(
    operation: str, discriminator: str, response: dict[str, object]
) -> MatrixCapacityFailure:
    """保留服务端最后一次 429 细节，避免容量失败只剩一句废话。"""

    errcode = response.get("errcode", "unknown")
    retry_after_ms = response.get("retry_after_ms", "unknown")
    return MatrixCapacityFailure(
        f"{operation}持续被 Homeserver 限流：目标={discriminator}，"
        f"尝试={RATE_LIMIT_ATTEMPTS}，errcode={errcode}，"
        f"retry_after_ms={retry_after_ms}。"
    )


def send_sustained(
    owner: MatrixUser,
    room_id: str,
    duration_seconds: int,
    *,
    rate: int = SUSTAINED_RATE,
    workers: int | None = None,
    sender: CapacityMessageSender | None = None,
) -> tuple[list[str], list[float], float]:
    if rate <= 0:
        raise MatrixCapacityFailure("持续消息速率必须为正数。")
    send = sender or send_capacity_message
    worker_count = workers or max(4, rate * 2)
    futures = []
    started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        for ordinal in range(duration_seconds * rate):
            deadline = started + ordinal / rate
            delay = deadline - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            futures.append(
                executor.submit(send, owner, room_id, f"sustained-{ordinal}")
            )
        results = [future.result() for future in futures]
    elapsed = time.perf_counter() - started
    return [result[0] for result in results], [result[1] for result in results], elapsed


def send_burst(owner: MatrixUser, room_id: str) -> tuple[list[str], list[float], float]:
    started = time.perf_counter()
    results = parallel_map(
        lambda ordinal: send_capacity_message(owner, room_id, f"burst-{ordinal}"),
        range(BURST_COUNT),
        workers=25,
    )
    elapsed = time.perf_counter() - started
    return [result[0] for result in results], [result[1] for result in results], elapsed


def observed_event_ids(user: MatrixUser, room_id: str, maximum_pages: int = 20) -> set[str]:
    observed: set[str] = set()
    token: str | None = None
    for _ in range(maximum_pages):
        query: dict[str, str | int] = {"dir": "b", "limit": 100}
        if token is not None:
            query["from"] = token
        response = matrix_request(
            user.peer,
            "GET",
            f"/_matrix/client/v3/rooms/{encoded(room_id)}/messages?{urlencode(query)}",
            token=user.access_token,
        )
        chunk = response.get("chunk")
        if not isinstance(chunk, list):
            raise MatrixCapacityFailure("房间消息响应缺少 chunk。")
        for event in chunk:
            if not isinstance(event, dict):
                continue
            if event.get("type") != "io.github.rainyflash.agentroom.message.preview.v1":
                continue
            event_id = event.get("event_id")
            if isinstance(event_id, str):
                observed.add(event_id)
        next_token = response.get("end")
        if not isinstance(next_token, str) or not chunk:
            break
        token = next_token
    return observed


def execute(sustained_seconds: int, *, keep_running: bool) -> dict[str, object]:
    if sustained_seconds <= 0:
        raise MatrixCapacityFailure("持续消息时长必须为正数。")
    revision = git_revision()
    down(volumes=True)
    prepare()
    try:
        up()
        values = read_environment()
        suffix = uuid.uuid4().hex[:8]
        registration_started = time.perf_counter()
        users = parallel_map(
            lambda ordinal: register_user(
                ALPHA,
                values["ALPHA_REGISTRATION_SECRET"],
                f"capacity-{suffix}-{ordinal:03d}",
                values["ALPHA_USER_PASSWORD"],
                administrator=ordinal == 0,
            ),
            range(MEMBER_COUNT),
            workers=REGISTRATION_WORKERS,
        )
        registration_seconds = time.perf_counter() - registration_started
        owner = next(user for user in users if user.user_id.endswith("-000:alpha.agent-room.test"))
        members = [user for user in users if user != owner]
        room_id = create_public_room(owner)

        join_started = time.perf_counter()
        join_latencies = parallel_map(
            lambda ordinal: join_room(members[ordinal], room_id),
            range(len(members)),
            workers=JOIN_WORKERS,
        )
        join_seconds = time.perf_counter() - join_started
        actual_members = joined_member_count(owner, room_id)

        state_started = time.perf_counter()
        state_latencies = parallel_map(
            lambda ordinal: renew_state(owner, room_id, ordinal),
            range(MEMBER_COUNT),
            workers=STATE_WORKERS,
        )
        state_seconds = time.perf_counter() - state_started

        sustained_ids, sustained_latencies, sustained_elapsed = send_sustained(
            owner, room_id, sustained_seconds
        )
        burst_ids, burst_latencies, burst_elapsed = send_burst(owner, room_id)
        expected_ids = set(sustained_ids + burst_ids)
        observed_ids = observed_event_ids(members[0], room_id)

        sustained_rate = len(sustained_ids) / sustained_elapsed
        burst_rate = len(burst_ids) / burst_elapsed
        delivery_ratio = len(expected_ids & observed_ids) / len(expected_ids)
        metrics: dict[str, object] = {
            "registeredMembers": len(users),
            "registrationSeconds": round(registration_seconds, 3),
            "joinedMembers": actual_members,
            "joinSeconds": round(join_seconds, 3),
            "joinP95Milliseconds": round(percentile(join_latencies, 0.95), 3),
            "leaseStateEvents": len(state_latencies),
            "leaseStateSeconds": round(state_seconds, 3),
            "leaseStateP95Milliseconds": round(percentile(state_latencies, 0.95), 3),
            "sustainedDurationSeconds": sustained_seconds,
            "sustainedMessages": len(sustained_ids),
            "sustainedMessagesPerSecond": round(sustained_rate, 3),
            "sustainedAckP95Milliseconds": round(percentile(sustained_latencies, 0.95), 3),
            "burstMessages": len(burst_ids),
            "burstElapsedSeconds": round(burst_elapsed, 3),
            "burstMessagesPerSecond": round(burst_rate, 3),
            "burstAckP95Milliseconds": round(percentile(burst_latencies, 0.95), 3),
            "uniqueAcceptedEvents": len(expected_ids),
            "observerDeliveryRatio": round(delivery_ratio, 4),
            "nextCapacityThreshold": {
                "members": 300,
                "sustainedMessagesPerSecond": 15,
                "burstMessagesPerSecond": 75,
            },
        }
        passed = (
            actual_members == MEMBER_COUNT
            and len(state_latencies) == MEMBER_COUNT
            and len(expected_ids) == sustained_seconds * SUSTAINED_RATE + BURST_COUNT
            and sustained_rate >= 9.5
            and burst_rate >= 50.0
            and delivery_ratio == 1.0
        )
        release_eligible = passed and sustained_seconds >= 60
        require_git_revision(revision)
        report: dict[str, object] = {
            "schemaVersion": 1,
            "scenario": "matrix_lobby_and_messages",
            "evidenceLevel": "real_synapse_client_api",
            "generatedAt": datetime.now(UTC).isoformat(),
            "revision": revision,
            "passed": passed,
            "releaseGateEligible": release_eligible,
            "topology": {
                "homeserver": ALPHA.server_name,
                "roomId": room_id,
                "members": MEMBER_COUNT,
            },
            "metrics": metrics,
        }
        write_json(REPORT, report)
        if not passed:
            raise MatrixCapacityFailure("真实 Matrix 容量指标没有达到设计目标。")
        if not release_eligible:
            raise MatrixCapacityFailure("持续消息观察不足 60 秒，不能用于发布放行。")
        return report
    finally:
        if not keep_running:
            down(volumes=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sustained-seconds", type=int, default=60)
    parser.add_argument("--keep-running", action="store_true")
    arguments = parser.parse_args()
    try:
        report = execute(arguments.sustained_seconds, keep_running=arguments.keep_running)
        print(f"Matrix 容量报告：{REPORT}")
        print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
        return 0
    except (CapacityFailure, FederationFailure, MatrixCapacityFailure) as error:
        print(str(error), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
