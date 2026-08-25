#!/usr/bin/env python3
"""在两个真实 Synapse 之间验证 30 分钟断网与有序回填。"""

from __future__ import annotations

import argparse
from datetime import UTC, datetime
import json
from pathlib import Path
import sys
import time
from typing import Final, Sequence
from urllib.parse import urlencode
import uuid

if __package__:
    from . import federation
    from .capacity import write_json
else:
    import federation
    from capacity import write_json


ROOT: Final = Path(__file__).resolve().parent.parent
REPORT_PATH: Final = ROOT / "artifacts" / "capacity" / "federation-report.json"
REQUIRED_OUTAGE_SECONDS: Final = 30 * 60
REQUIRED_EVENT_COUNT: Final = 10


class CapacityFederationFailure(RuntimeError):
    """表示真实联邦回填没有达到容量门槛。"""


def sync_with_timeline_limit(
    user: federation.MatrixUser,
    since: str | None,
    *,
    timeline_limit: int,
) -> dict[str, object]:
    query: dict[str, str | int] = {
        "filter": json.dumps(
            {"room": {"timeline": {"limit": timeline_limit}}},
            separators=(",", ":"),
        ),
        "timeout": 1_000,
    }
    if since is not None:
        query["since"] = since
    return federation.matrix_request(
        user.peer,
        "GET",
        f"/_matrix/client/v3/sync?{urlencode(query)}",
        token=user.access_token,
    )


def wait_for_ordered_backfill(
    user: federation.MatrixUser,
    room_id: str,
    expected_event_ids: Sequence[str],
    since: str,
    *,
    timeout_seconds: float = 300,
) -> tuple[list[str], list[str]]:
    expected = set(expected_event_ids)
    observed: list[str] = []
    duplicates: list[str] = []
    seen: set[str] = set()
    token = since
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        response = sync_with_timeline_limit(
            user,
            token,
            timeline_limit=max(100, len(expected_event_ids) * 2),
        )
        token = federation.next_batch(response)
        for event in federation.joined_room_events(response, room_id):
            event_id = event.get("event_id")
            if not isinstance(event_id, str) or event_id not in expected:
                continue
            if event_id in seen:
                duplicates.append(event_id)
                continue
            seen.add(event_id)
            observed.append(event_id)
        if seen == expected:
            return observed, duplicates
    missing = expected - seen
    raise CapacityFederationFailure(f"联邦恢复后仍缺少事件：{sorted(missing)}")


def delivery_assessment(
    expected_event_ids: Sequence[str],
    observed_event_ids: Sequence[str],
    duplicate_event_ids: Sequence[str],
    *,
    actual_outage_seconds: float,
    requested_outage_seconds: int,
) -> dict[str, object]:
    expected = list(expected_event_ids)
    observed = list(observed_event_ids)
    delivered = len(set(expected) & set(observed))
    delivery_ratio = delivered / len(expected) if expected else 0.0
    outage_reached = actual_outage_seconds + 0.1 >= requested_outage_seconds
    ordered = observed == expected
    no_duplicates = not duplicate_event_ids
    passed = (
        bool(expected)
        and delivery_ratio == 1.0
        and ordered
        and no_duplicates
        and outage_reached
    )
    release_eligible = (
        passed
        and requested_outage_seconds >= REQUIRED_OUTAGE_SECONDS
        and actual_outage_seconds >= REQUIRED_OUTAGE_SECONDS
        and len(expected) >= REQUIRED_EVENT_COUNT
    )
    return {
        "deliveryRatio": delivery_ratio,
        "deliveredEvents": delivered,
        "duplicateEvents": len(duplicate_event_ids),
        "eventOrderPreserved": ordered,
        "outageDurationReached": outage_reached,
        "passed": passed,
        "releaseGateEligible": release_eligible,
    }


def wait_until(started: float, target_seconds: float) -> None:
    while True:
        remaining = target_seconds - (time.monotonic() - started)
        if remaining <= 0:
            return
        time.sleep(min(1.0, remaining))


def execute(outage_seconds: int, event_count: int, *, keep_running: bool) -> dict[str, object]:
    if outage_seconds <= 0:
        raise CapacityFederationFailure("断网时长必须为正数。")
    if event_count <= 0 or event_count > 100:
        raise CapacityFederationFailure("回填事件数必须处于 1 到 100。")

    federation.down(volumes=True)
    federation.prepare()
    peer_stopped = False
    try:
        federation.up()
        values = federation.read_environment()
        suffix = uuid.uuid4().hex[:10]
        alpha = federation.register_user(
            federation.ALPHA,
            values["ALPHA_REGISTRATION_SECRET"],
            f"capacity-alpha-{suffix}",
            values["ALPHA_USER_PASSWORD"],
            administrator=True,
        )
        beta = federation.register_user(
            federation.BETA,
            values["BETA_REGISTRATION_SECRET"],
            f"capacity-beta-{suffix}",
            values["BETA_USER_PASSWORD"],
            administrator=False,
        )
        room_id = federation.create_room(alpha, beta, alias_prefix="capacity-backfill")
        federation.wait_for_room(beta, room_id, "invite")
        federation.join_remote_room(beta, room_id, federation.ALPHA)
        beta_since = federation.wait_for_room(beta, room_id, "join")
        local_room_id = federation.create_room(
            alpha, None, alias_prefix="capacity-local-survival"
        )

        federation.stop_peer(federation.BETA)
        peer_stopped = True
        outage_started = time.monotonic()

        local_write_started = time.monotonic()
        local_event_id, local_event_accepted_at = federation.send_event(
            alpha,
            local_room_id,
            "local write while beta is unavailable",
        )
        local_write_milliseconds = (time.monotonic() - local_write_started) * 1_000

        expected_event_ids: list[str] = []
        accepted_at: dict[str, str] = {}
        for index in range(event_count):
            wait_until(outage_started, outage_seconds * index / event_count)
            event_id, timestamp = federation.send_event(
                alpha,
                room_id,
                f"capacity outage message {index + 1:03d}",
            )
            expected_event_ids.append(event_id)
            accepted_at[event_id] = timestamp
        wait_until(outage_started, float(outage_seconds))
        actual_outage_seconds = time.monotonic() - outage_started

        restart_started = time.monotonic()
        federation.start_peer(federation.BETA)
        peer_stopped = False
        restart_milliseconds = (time.monotonic() - restart_started) * 1_000
        backfill_started = time.monotonic()
        observed_event_ids, duplicate_event_ids = wait_for_ordered_backfill(
            beta,
            room_id,
            expected_event_ids,
            beta_since,
        )
        backfill_milliseconds = (time.monotonic() - backfill_started) * 1_000
        assessment = delivery_assessment(
            expected_event_ids,
            observed_event_ids,
            duplicate_event_ids,
            actual_outage_seconds=actual_outage_seconds,
            requested_outage_seconds=outage_seconds,
        )
        report: dict[str, object] = {
            "schemaVersion": 1,
            "scenario": "federation_outage_backfill",
            "evidenceLevel": "real_service",
            "generatedAt": datetime.now(UTC).isoformat(),
            "revision": federation.git_revision(),
            "passed": assessment["passed"],
            "releaseGateEligible": assessment["releaseGateEligible"],
            "topology": federation.diagnose(),
            "metrics": {
                **assessment,
                "requestedOutageSeconds": outage_seconds,
                "actualOutageSeconds": round(actual_outage_seconds, 3),
                "queuedEvents": event_count,
                "localWriteMilliseconds": round(local_write_milliseconds, 3),
                "peerRestartMilliseconds": round(restart_milliseconds, 3),
                "backfillMilliseconds": round(backfill_milliseconds, 3),
            },
            "evidence": {
                "roomId": room_id,
                "localRoomId": local_room_id,
                "localEventId": local_event_id,
                "localEventAcceptedAt": local_event_accepted_at,
                "queuedEventAcceptedAt": accepted_at,
                "expectedEventOrder": expected_event_ids,
                "observedEventOrder": observed_event_ids,
                "duplicateEventIds": duplicate_event_ids,
            },
            "nextCapacityThreshold": {
                "outageSeconds": 60 * 60,
                "queuedEvents": 100,
            },
        }
        write_json(REPORT_PATH, report)
        return report
    finally:
        if peer_stopped:
            try:
                federation.start_peer(federation.BETA)
            except federation.FederationFailure:
                pass
        if not keep_running:
            federation.down(volumes=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outage-seconds", type=int, default=REQUIRED_OUTAGE_SECONDS)
    parser.add_argument("--events", type=int, default=REQUIRED_EVENT_COUNT)
    parser.add_argument("--keep-running", action="store_true")
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    try:
        report = execute(
            arguments.outage_seconds,
            arguments.events,
            keep_running=arguments.keep_running,
        )
        print(f"联邦容量报告：{REPORT_PATH}")
        print(json.dumps(report["metrics"], ensure_ascii=False, indent=2))
        return 0 if report["passed"] is True else 1
    except (CapacityFederationFailure, federation.FederationFailure) as error:
        print(f"联邦容量测试失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
