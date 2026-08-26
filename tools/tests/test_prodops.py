from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest
import uuid

from tools.prodops.config import DeploymentConfig, DeploymentConfigError, load_deployment_config
from tools.prodops.render import DeploymentPaths, render_deployment
from tools.prodops.runtime import ProductionRuntime
from tools.prodops.secrets import SecretStore


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
EXTERNAL_EXAMPLE = ROOT / "infra" / "production" / "deployment.external.example.json"
SCHEMA = ROOT / "infra" / "production" / "deployment.schema.json"


class ProductionConfigTests(unittest.TestCase):
    def test_example_matches_strict_domain_model(self) -> None:
        config = load_deployment_config(EXAMPLE)

        self.assertEqual(config.schema_version, 1)
        self.assertEqual(config.database.mode, "embedded")
        self.assertEqual(config.compose_profiles, ("embedded-database", "embedded-object-store"))
        self.assertEqual(json.loads(SCHEMA.read_text(encoding="utf-8"))["$schema"], "https://json-schema.org/draft/2020-12/schema")

    def test_unknown_configuration_is_rejected(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["unsafeDefault"] = True

        with self.assertRaises(DeploymentConfigError):
            DeploymentConfig.from_mapping(value)

    def test_external_database_requires_transport_security(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["database"] = {
            "mode": "external",
            "host": "postgres.agent-room.example",
            "port": 5432,
            "tlsMode": "disable",
        }

        with self.assertRaises(DeploymentConfigError):
            DeploymentConfig.from_mapping(value)

    def test_external_scale_example_disables_embedded_profiles(self) -> None:
        config = load_deployment_config(EXTERNAL_EXAMPLE)

        self.assertEqual(config.compose_profiles, ())
        self.assertEqual(config.capacity.control_plane_replicas, 2)
        self.assertEqual(config.capacity.synapse_workers, 2)

    def test_service_domains_must_be_distinct(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["public"]["apiDomain"] = value["public"]["appDomain"]

        with self.assertRaises(DeploymentConfigError):
            DeploymentConfig.from_mapping(value)


class ProductionRenderingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.state = Path(self.temporary.name) / "agent-room"
        self.paths = DeploymentPaths.from_state(self.state)
        self.config = load_deployment_config(EXAMPLE)
        self.secrets = SecretStore(self.paths.secrets)
        self.secrets.initialize()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_secrets_are_stable_and_uuidv7_is_generated(self) -> None:
        first = self.secrets.read("agent_room_db_runtime_password")
        self.secrets.initialize()

        self.assertEqual(self.secrets.read("agent_room_db_runtime_password"), first)
        self.assertEqual(uuid.UUID(self.secrets.read("content_matrix_agent_id")).version, 7)

    def test_render_keeps_secret_values_out_of_compose_environment(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        environment = self.paths.compose_environment.read_text(encoding="utf-8")
        runtime_password = self.secrets.read("agent_room_db_runtime_password")

        self.assertNotIn(runtime_password, environment)
        self.assertIn("COMPOSE_PROFILES=embedded-database,embedded-object-store,telemetry", environment)
        self.assertIn("AGENT_ROOM_CONTENT_S3_CREATE_BUCKET=true", environment)
        self.assertIn("sslmode=disable", self.secrets.read("migration_database_url"))

    def test_rendered_identity_and_synapse_share_the_same_oidc_contract(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        realm = json.loads(
            self.paths.generated.joinpath("keycloak", "realm-agent-room.json").read_text(encoding="utf-8")
        )
        homeserver = self.paths.generated.joinpath("synapse", "homeserver.yaml").read_text(encoding="utf-8")

        matrix_client = next(client for client in realm["clients"] if client["clientId"] == "agent-room-matrix")
        self.assertIn(matrix_client["secret"], homeserver)
        self.assertIn(self.config.public.identity_origin, homeserver)

    def test_worker_count_generates_unique_processes_and_routes(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["capacity"]["synapseWorkers"] = 2
        config = DeploymentConfig.from_mapping(value)

        render_deployment(config, self.paths, self.secrets)
        override = json.loads(self.paths.worker_override.read_text(encoding="utf-8"))
        caddyfile = self.paths.generated.joinpath("caddy", "Caddyfile").read_text(encoding="utf-8")

        self.assertEqual(set(override["services"]), {"synapse-worker-1", "synapse-worker-2"})
        self.assertIn("synapse-worker-1:8083 synapse-worker-2:8083", caddyfile)

    def test_compose_command_uses_generated_override_and_environment(self) -> None:
        runtime = ProductionRuntime(self.config, self.paths)
        command = runtime.compose_command()

        self.assertIn(str(self.paths.compose_environment), command)
        self.assertIn(str(self.paths.worker_override), command)

    def test_external_object_store_is_verified_without_creation_rights(self) -> None:
        config = load_deployment_config(EXTERNAL_EXAMPLE)

        render_deployment(config, self.paths, self.secrets)
        environment = self.paths.compose_environment.read_text(encoding="utf-8")

        self.assertIn("AGENT_ROOM_CONTENT_S3_CREATE_BUCKET=false", environment)


if __name__ == "__main__":
    unittest.main()
