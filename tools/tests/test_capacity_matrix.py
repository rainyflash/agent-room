from __future__ import annotations

import unittest
from threading import Event, Lock

from tools.capacity_matrix import (
    JOIN_WORKERS,
    RATE_LIMIT_ATTEMPTS,
    MatrixUser,
    parallel_map,
    rate_limit_exhausted,
    retry_delay,
    send_sustained,
)
from tools.federation import ALPHA


class MatrixCapacityUtilitiesTests(unittest.TestCase):
    def test_parallel_map_returns_every_result_without_order_assumption(self) -> None:
        results = parallel_map(lambda value: value * value, range(20), workers=4)
        self.assertEqual(sorted(results), [value * value for value in range(20)])

    def test_sustained_sender_does_not_serialize_acknowledgements(self) -> None:
        overlap = Event()
        lock = Lock()
        active = 0

        def send(_owner: MatrixUser, _room_id: str, label: str) -> tuple[str, float]:
            nonlocal active
            with lock:
                active += 1
                if active >= 2:
                    overlap.set()
            if not overlap.wait(timeout=0.4):
                raise AssertionError("持续发送器把 Matrix ACK 串行化了。")
            with lock:
                active -= 1
            return f"${label}", 10.0

        owner = MatrixUser(ALPHA, "@owner:alpha.agent-room.test", "token", "password")
        event_ids, latencies, elapsed = send_sustained(
            owner,
            "!room:alpha.agent-room.test",
            1,
            rate=4,
            workers=2,
            sender=send,
        )

        self.assertEqual(
            event_ids,
            ["$sustained-0", "$sustained-1", "$sustained-2", "$sustained-3"],
        )
        self.assertEqual(latencies, [10.0, 10.0, 10.0, 10.0])
        self.assertGreaterEqual(elapsed, 0.75)
        self.assertLess(elapsed, 1.2)

    def test_rate_limit_retry_honors_server_and_staggers_callers(self) -> None:
        response: dict[str, object] = {"retry_after_ms": 8_000}

        first = retry_delay(response, 3, "@first:example.test")
        repeated = retry_delay(response, 3, "@first:example.test")
        second = retry_delay(response, 3, "@second:example.test")

        self.assertEqual(first, repeated)
        self.assertNotEqual(first, second)
        self.assertGreaterEqual(first, 8.0)
        self.assertLessEqual(first, 8.4)
        self.assertGreaterEqual(RATE_LIMIT_ATTEMPTS, 24)

    def test_join_concurrency_is_bounded_and_failures_keep_server_context(self) -> None:
        self.assertLessEqual(JOIN_WORKERS, 8)
        error = rate_limit_exhausted(
            "加入大厅",
            "@agent:example.test",
            {"errcode": "M_LIMIT_EXCEEDED", "retry_after_ms": 12_000},
        )

        self.assertIn("@agent:example.test", str(error))
        self.assertIn("M_LIMIT_EXCEEDED", str(error))
        self.assertIn("12000", str(error))


if __name__ == "__main__":
    unittest.main()
