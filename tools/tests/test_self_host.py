from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from tools.prodops.config import DeploymentConfig
from tools.prodops.self_host import SelfHostConfig, SelfHostConfigError, write_new_config


class SelfHostConfigTests(unittest.TestCase):
    def test_embedded_config_derives_distinct_public_domains(self) -> None:
        document = SelfHostConfig(
            domain="ROOM.EXAMPLE.COM.",
        ).document()

        public = document["public"]
        self.assertIsInstance(public, dict)
        assert isinstance(public, dict)
        self.assertEqual(public["serverName"], "room.example.com")
        self.assertEqual(public["appDomain"], "app.room.example.com")
        self.assertNotIn("acmeEmail", public)
        self.assertEqual(document["telemetry"], {"enabled": False})
        parsed = DeploymentConfig.from_mapping(document)
        self.assertIsNone(parsed.public.acme_email)
        self.assertEqual(parsed.compose_profiles, ("embedded-database", "embedded-object-store"))

    def test_optional_acme_email_is_preserved_when_supplied(self) -> None:
        document = SelfHostConfig(
            domain="room.example.com",
            acme_email="operator@example.com",
        ).document()

        public = document["public"]
        self.assertIsInstance(public, dict)
        assert isinstance(public, dict)
        self.assertEqual(public["acmeEmail"], "operator@example.com")
        self.assertEqual(
            DeploymentConfig.from_mapping(document).public.acme_email,
            "operator@example.com",
        )

    def test_external_dependencies_require_complete_secure_inputs(self) -> None:
        config = SelfHostConfig(
            domain="room.example.com",
            acme_email="operator@example.com",
            database_mode="external",
            object_store_mode="external",
        )

        with self.assertRaises(SelfHostConfigError):
            config.document()

    def test_external_config_is_accepted_when_inputs_are_complete(self) -> None:
        document = SelfHostConfig(
            domain="room.example.com",
            acme_email="operator@example.com",
            database_mode="external",
            database_host="postgres.room.example.com",
            provider_pitr_evidence_file="/etc/agent-room/pitr.json",
            object_store_mode="external",
            object_store_endpoint="https://objects.room.example.com",
            object_store_health_url="https://objects.room.example.com/healthz",
            control_plane_replicas=2,
            synapse_workers=2,
            alert_webhook_url="https://alerts.room.example.com/events",
        ).document()

        parsed = DeploymentConfig.from_mapping(document)
        self.assertEqual(parsed.compose_profiles, ())
        self.assertTrue(parsed.telemetry_enabled)

    def test_writer_is_private_and_never_overwrites(self) -> None:
        config = SelfHostConfig(
            domain="room.example.com",
            acme_email="operator@example.com",
        )
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "config" / "deployment.json"
            write_new_config(config, output)
            original = output.read_bytes()

            with self.assertRaises(SelfHostConfigError):
                write_new_config(config, output)

            self.assertEqual(output.read_bytes(), original)
            self.assertEqual(json.loads(original)["schemaVersion"], 1)


if __name__ == "__main__":
    unittest.main()
