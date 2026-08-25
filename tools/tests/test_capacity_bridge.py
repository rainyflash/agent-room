from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

from tools.capacity import CapacityFailure
from tools.capacity_bridge import runtime_environment


class BridgeCapacityLauncherTests(unittest.TestCase):
    def test_agent_and_catalog_configuration_is_atomic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            data_root = Path(directory).resolve()
            with self.assertRaisesRegex(CapacityFailure, "必须同时提供"):
                runtime_environment(data_root, "018f0000-0000-7000-8000-000000000001", None)

            environment = runtime_environment(data_root, None, None)

        self.assertNotIn("AGENT_ROOM_AGENT_ID", environment)
        self.assertNotIn("AGENT_ROOM_PUBLIC_LOBBY_CATALOG_ID", environment)
        self.assertEqual(
            environment["AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE"],
            "agent-room-capacity-bridge-v1",
        )


if __name__ == "__main__":
    unittest.main()
