from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch
import uuid

from tools.prodops.config import DeploymentConfig, DeploymentConfigError, load_deployment_config
from tools.prodops.render import (
    CONTAINER_CONFIG_DIRECTORY_MODE,
    CONTAINER_CONFIG_FILE_MODE,
    PRIVATE_DIRECTORY_MODE,
    DeploymentPaths,
    _generated_config_digest,
    render_deployment,
)
from tools.prodops.runtime import ProductionRuntime, _meets_nominal_memory
from tools.prodops.secrets import (
    CONTAINER_SECRET_FILE_MODE,
    SECRET_DIRECTORY_MODE,
    SECRET_NAMES,
    SecretStore,
)


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "infra" / "production" / "deployment.example.json"
EXTERNAL_EXAMPLE = ROOT / "infra" / "production" / "deployment.external.example.json"
SCHEMA = ROOT / "infra" / "production" / "deployment.schema.json"


class ProductionConfigTests(unittest.TestCase):
    def test_example_matches_strict_domain_model(self) -> None:
        config = load_deployment_config(EXAMPLE)

        self.assertEqual(config.schema_version, 1)
        self.assertEqual(config.database.mode, "embedded")
        self.assertEqual(config.backup.rpo_minutes, 15)
        self.assertEqual(config.backup.archive_timeout_seconds, 900)
        self.assertEqual(
            config.telemetry.alert_webhook_url,
            "https://alerts.agent-room.example/v1/events",
        )
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

    def test_external_database_requires_provider_pitr_evidence(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["database"] = {
            "mode": "external",
            "host": "postgres.agent-room.example",
            "port": 5432,
            "tlsMode": "verify-full",
        }

        with self.assertRaisesRegex(DeploymentConfigError, "PITR"):
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

    def test_acme_email_is_optional_but_validated_when_present(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        self.assertIsNone(DeploymentConfig.from_mapping(value).public.acme_email)

        value["public"]["acmeEmail"] = "not-an-email"
        with self.assertRaisesRegex(DeploymentConfigError, "acmeEmail"):
            DeploymentConfig.from_mapping(value)

    def test_enabled_telemetry_requires_secret_free_https_receiver(self) -> None:
        missing = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        missing["telemetry"].pop("alertWebhookUrl")
        insecure = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        insecure["telemetry"]["alertWebhookUrl"] = "http://alerts.example/events"

        with self.assertRaises(DeploymentConfigError):
            DeploymentConfig.from_mapping(missing)
        with self.assertRaises(DeploymentConfigError):
            DeploymentConfig.from_mapping(insecure)


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

    def test_derived_secret_write_is_idempotent_and_replaceable(self) -> None:
        self.secrets.write_derived("migration_database_url", "first")
        self.secrets.write_derived("migration_database_url", "first")
        self.secrets.write_derived("migration_database_url", "second")

        self.assertEqual(self.secrets.read("migration_database_url"), "second")

    @unittest.skipIf(os.name == "nt", "Windows 不提供生产 POSIX 权限语义")
    def test_container_secrets_are_read_only_inside_private_directory(self) -> None:
        self.assertEqual(stat.S_IMODE(self.paths.secrets.stat().st_mode), SECRET_DIRECTORY_MODE)
        for name in (*SECRET_NAMES, "content_matrix_agent_id"):
            self.assertEqual(
                stat.S_IMODE(self.secrets.path(name).stat().st_mode),
                CONTAINER_SECRET_FILE_MODE,
            )

    def test_render_keeps_secret_values_out_of_compose_environment(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        environment = self.paths.compose_environment.read_text(encoding="utf-8")
        runtime_password = self.secrets.read("agent_room_db_runtime_password")

        self.assertNotIn(runtime_password, environment)
        self.assertIn("COMPOSE_PROFILES=embedded-database,embedded-object-store,telemetry", environment)
        self.assertIn("AGENT_ROOM_CONTENT_S3_CREATE_BUCKET=true", environment)
        self.assertIn("AGENT_ROOM_BACKUP_ARCHIVE_TIMEOUT_SECONDS=900", environment)
        digest_line = next(
            line
            for line in environment.splitlines()
            if line.startswith("AGENT_ROOM_GENERATED_CONFIG_DIGEST=")
        )
        self.assertRegex(digest_line.removeprefix("AGENT_ROOM_GENERATED_CONFIG_DIGEST="), r"^[0-9a-f]{64}$")
        self.assertIn("sslmode=disable", self.secrets.read("migration_database_url"))

    def test_generated_config_digest_is_stable_and_changes_with_content(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        first = self.paths.compose_environment.read_text(encoding="utf-8")
        render_deployment(self.config, self.paths, self.secrets)
        second = self.paths.compose_environment.read_text(encoding="utf-8")

        self.assertEqual(first, second)
        caddyfile = self.paths.generated / "caddy" / "Caddyfile"
        caddyfile.chmod(0o600)
        caddyfile.write_text(
            caddyfile.read_text(encoding="utf-8") + "\n# 配置变更\n",
            encoding="utf-8",
        )
        first_digest = next(
            line.split("=", 1)[1]
            for line in first.splitlines()
            if line.startswith("AGENT_ROOM_GENERATED_CONFIG_DIGEST=")
        )
        self.assertNotEqual(_generated_config_digest(self.paths), first_digest)

    def test_render_omits_unconfigured_acme_contact(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        environment = self.paths.compose_environment.read_text(encoding="utf-8")
        caddyfile = self.paths.generated.joinpath("caddy", "Caddyfile").read_text(
            encoding="utf-8"
        )

        self.assertNotIn("AGENT_ROOM_ACME_EMAIL", environment)
        self.assertNotIn("\temail ", caddyfile)
        self.assertIn("\tadmin off", caddyfile)

    def test_prepare_keeps_postgres_18_mount_parent_traversable(self) -> None:
        with patch.object(Path, "chmod", autospec=True) as chmod:
            self.paths.prepare()

        chmod.assert_any_call(self.paths.data / "postgres", 0o711)

    @unittest.skipIf(os.name == "nt", "Windows 不提供生产 POSIX 权限语义")
    def test_rendered_configuration_is_read_only_to_non_root_containers(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)

        self.assertEqual(
            stat.S_IMODE(self.paths.generated.stat().st_mode),
            PRIVATE_DIRECTORY_MODE,
        )
        for root in self.paths.container_config_directories():
            self.assertEqual(
                stat.S_IMODE(root.stat().st_mode),
                CONTAINER_CONFIG_DIRECTORY_MODE,
            )
            for child in root.rglob("*"):
                expected = (
                    CONTAINER_CONFIG_DIRECTORY_MODE
                    if child.is_dir()
                    else CONTAINER_CONFIG_FILE_MODE
                )
                self.assertEqual(stat.S_IMODE(child.stat().st_mode), expected)

        # 第二次渲染必须先恢复目录写权限，升级不能被自己的只读策略卡死。
        render_deployment(self.config, self.paths, self.secrets)

    def test_observability_render_has_paging_and_fixed_probe_names(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        prometheus = self.paths.generated.joinpath(
            "observability", "prometheus.yaml"
        ).read_text(encoding="utf-8")
        alertmanager = self.paths.generated.joinpath(
            "observability", "alertmanager.yaml"
        ).read_text(encoding="utf-8")

        self.assertIn('probe_name: "federation-version"', prometheus)
        self.assertIn("alertmanager:9093", prometheus)
        self.assertIn("/run/secrets/alertmanager_webhook_token", alertmanager)
        self.assertIn(self.config.telemetry.alert_webhook_url or "", alertmanager)
        self.assertNotIn(self.secrets.read("alertmanager_webhook_token"), alertmanager)

    def test_disabled_telemetry_does_not_render_observability_secrets(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["telemetry"] = {"enabled": False}
        config = DeploymentConfig.from_mapping(value)

        render_deployment(config, self.paths, self.secrets)

        self.assertFalse(self.paths.generated.joinpath("observability", "prometheus.yaml").exists())
        self.assertNotIn("telemetry", self.paths.compose_environment.read_text(encoding="utf-8"))

    def test_rendered_identity_and_synapse_share_the_same_oidc_contract(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        realm = json.loads(
            self.paths.generated.joinpath("keycloak", "realm-agent-room.json").read_text(encoding="utf-8")
        )
        homeserver = self.paths.generated.joinpath("synapse", "homeserver.yaml").read_text(encoding="utf-8")

        matrix_client = next(client for client in realm["clients"] if client["clientId"] == "agent-room-matrix")
        self.assertIn(matrix_client["secret"], homeserver)
        self.assertIn(self.config.public.identity_origin, homeserver)
        self.assertNotIn("http://identity", homeserver)
        for endpoint in ("auth", "token", "userinfo", "certs"):
            self.assertIn(
                f"{self.config.public.identity_origin}/realms/agent-room/protocol/openid-connect/{endpoint}",
                homeserver,
            )
        self.assertIn("retention:\n  enabled: true", homeserver)
        self.assertIn("max_lifetime: 30d", homeserver)

    def test_worker_count_generates_unique_processes_and_routes(self) -> None:
        value = json.loads(EXAMPLE.read_text(encoding="utf-8"))
        value["capacity"]["synapseWorkers"] = 2
        config = DeploymentConfig.from_mapping(value)

        render_deployment(config, self.paths, self.secrets)
        override = json.loads(self.paths.worker_override.read_text(encoding="utf-8"))
        caddyfile = self.paths.generated.joinpath("caddy", "Caddyfile").read_text(encoding="utf-8")

        self.assertEqual(set(override["services"]), {"synapse-worker-1", "synapse-worker-2"})
        self.assertIn("synapse-worker-1:8083 synapse-worker-2:8083", caddyfile)
        self.assertEqual(
            override["services"]["synapse-worker-1"]["labels"][
                "org.agent-room.generated-config-digest"
            ],
            "${AGENT_ROOM_GENERATED_CONFIG_DIGEST:?缺少生成配置摘要}",
        )

    def test_compose_command_uses_generated_override_and_environment(self) -> None:
        runtime = ProductionRuntime(self.config, self.paths)
        command = runtime.compose_command()

        self.assertIn(str(self.paths.compose_environment), command)
        self.assertIn(str(self.paths.worker_override), command)

    def test_nominal_memory_check_allows_only_bounded_system_reservation(self) -> None:
        gibibyte = 1024**3
        allowance = 256 * 1024**2

        self.assertTrue(_meets_nominal_memory(4 * gibibyte - allowance, 4 * gibibyte))
        self.assertFalse(_meets_nominal_memory(4 * gibibyte - allowance - 1, 4 * gibibyte))
        self.assertTrue(_meets_nominal_memory(8 * gibibyte - allowance, 8 * gibibyte))

    def test_embedded_database_allows_only_backup_role_to_replicate(self) -> None:
        rules = ROOT.joinpath("infra", "production", "postgres-pg-hba.conf").read_text(
            encoding="utf-8"
        )
        compose = ROOT.joinpath("infra", "production", "compose.yaml").read_text(encoding="utf-8")

        self.assertIn("replication     agent_room_bootstrap", rules)
        self.assertNotIn("replication     all", rules)
        self.assertIn("hba_file=/etc/postgresql/agent-room-pg-hba.conf", compose)

    def test_projection_rebuild_uses_server_side_copy(self) -> None:
        script = ROOT.joinpath("infra", "production", "projection-rebuild.sh").read_text(
            encoding="utf-8"
        )

        self.assertIn("COPY (", script)
        self.assertIn("COPY restored_matrix_membership FROM", script)
        self.assertNotIn("\\copy", script)

    def test_external_object_store_is_verified_without_creation_rights(self) -> None:
        config = load_deployment_config(EXTERNAL_EXAMPLE)

        render_deployment(config, self.paths, self.secrets)
        environment = self.paths.compose_environment.read_text(encoding="utf-8")

        self.assertIn("AGENT_ROOM_CONTENT_S3_CREATE_BUCKET=false", environment)

    def test_account_lifecycle_admin_is_provisioned_without_public_admin_api(self) -> None:
        render_deployment(self.config, self.paths, self.secrets)
        compose = ROOT.joinpath("infra", "production", "compose.yaml").read_text(
            encoding="utf-8"
        )
        caddyfile = self.paths.generated.joinpath("caddy", "Caddyfile").read_text(
            encoding="utf-8"
        )

        self.assertIn("synapse-admin-bootstrap:", compose)
        self.assertIn(
            "AGENT_ROOM_MATRIX_ADMIN_ACCESS_TOKEN_FILE: /run/secrets/synapse_lifecycle_admin_token",
            compose,
        )
        self.assertTrue(self.secrets.read("synapse_lifecycle_admin_password"))
        self.assertIn("synapse_lifecycle_admin_password", compose)
        self.assertLess(
            caddyfile.index("respond /_synapse/admin/* 404"),
            caddyfile.index("reverse_proxy synapse:8008"),
        )

    def test_synapse_shared_secret_registration_mac_matches_protocol(self) -> None:
        script = ROOT / "infra" / "production" / "synapse-admin-bootstrap.py"
        specification = importlib.util.spec_from_file_location(
            "agent_room_synapse_admin_bootstrap", script
        )
        self.assertIsNotNone(specification)
        self.assertIsNotNone(specification.loader if specification else None)
        module = importlib.util.module_from_spec(specification)
        assert specification is not None and specification.loader is not None
        specification.loader.exec_module(module)

        self.assertEqual(
            module.registration_mac("secret", "nonce", "alice", "password", True),
            "013c5738fc920e1110110046fc346bb5e30c53f2",
        )
        self.assertFalse(module.USERNAME.startswith("_"))
        self.assertEqual(
            module.matrix_error_summary(
                {
                    "errcode": "M_INVALID_USERNAME",
                    "error": "User ID may not begin with _\n",
                }
            ),
            "（M_INVALID_USERNAME：User ID may not begin with _）",
        )

    @unittest.skipIf(os.name == "nt", "Windows 不提供生产 POSIX 权限语义")
    def test_synapse_admin_token_is_materialized_as_container_secret(self) -> None:
        script = ROOT / "infra" / "production" / "synapse-admin-bootstrap.py"
        specification = importlib.util.spec_from_file_location(
            "agent_room_synapse_admin_bootstrap_mode",
            script,
        )
        self.assertIsNotNone(specification)
        self.assertIsNotNone(specification.loader if specification else None)
        module = importlib.util.module_from_spec(specification)
        assert specification is not None and specification.loader is not None
        specification.loader.exec_module(module)
        token_file = self.state / "derived" / "synapse_lifecycle_admin_token"
        module.TOKEN_FILE = token_file

        module.write_token("opaque-token")

        self.assertEqual(token_file.read_text(encoding="utf-8"), "opaque-token\n")
        self.assertEqual(
            stat.S_IMODE(token_file.stat().st_mode),
            CONTAINER_SECRET_FILE_MODE,
        )


if __name__ == "__main__":
    unittest.main()
