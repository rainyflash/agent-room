from __future__ import annotations

import unittest
from threading import Event, Lock

from tools.capacity_matrix import MatrixUser, parallel_map, send_sustained
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


if __name__ == "__main__":
    unittest.main()
