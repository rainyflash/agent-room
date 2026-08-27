"""自托管配置生成领域模型。"""

from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
from typing import Final

from .config import DeploymentConfig, DeploymentConfigError


DEFAULT_BACKUP_REPOSITORY: Final = "/var/backups/agent-room"


class SelfHostConfigError(ValueError):
    """表示自托管配置无法安全生成或落盘。"""


@dataclass(frozen=True, slots=True)
class SelfHostConfig:
    """生成生产部署文档所需的运营者输入。"""

    domain: str
    acme_email: str | None = None
    project_name: str = "agent-room"
    backup_repository: str = DEFAULT_BACKUP_REPOSITORY
    retention_days: int = 30
    rpo_minutes: int = 15
    database_mode: str = "embedded"
    database_host: str | None = None
    database_port: int = 5432
    database_tls_mode: str | None = None
    provider_pitr_evidence_file: str | None = None
    object_store_mode: str = "embedded"
    object_store_endpoint: str | None = None
    object_store_health_url: str | None = None
    object_store_bucket: str = "agent-room-content"
    object_store_region: str = "us-east-1"
    control_plane_replicas: int = 1
    synapse_workers: int = 0
    alert_webhook_url: str | None = None

    def document(self) -> dict[str, object]:
        """构造并通过正式领域解析器验证部署文档。"""

        document: dict[str, object] = {
            "schemaVersion": 1,
            "projectName": self.project_name,
            "public": self._public_document(),
            "database": self._database_document(),
            "objectStore": self._object_store_document(),
            "capacity": {
                "controlPlaneReplicas": self.control_plane_replicas,
                "synapseWorkers": self.synapse_workers,
            },
            "backup": self._backup_document(),
            "telemetry": self._telemetry_document(),
        }
        try:
            DeploymentConfig.from_mapping(document)
        except DeploymentConfigError as error:
            raise SelfHostConfigError(str(error)) from error
        return document

    def _public_document(self) -> dict[str, object]:
        base_domain = self.domain.strip().lower().rstrip(".")
        document: dict[str, object] = {
            "serverName": base_domain,
            "appDomain": f"app.{base_domain}",
            "apiDomain": f"api.{base_domain}",
            "matrixDomain": f"matrix.{base_domain}",
            "identityDomain": f"id.{base_domain}",
        }
        if self.acme_email is not None:
            document["acmeEmail"] = self.acme_email
        return document

    def _database_document(self) -> dict[str, object]:
        document: dict[str, object] = {
            "mode": self.database_mode,
            "port": self.database_port,
        }
        if self.database_mode == "embedded":
            document["tlsMode"] = self.database_tls_mode or "disable"
        else:
            if self.database_host is not None:
                document["host"] = self.database_host
            document["tlsMode"] = self.database_tls_mode or "verify-full"
        return document

    def _object_store_document(self) -> dict[str, object]:
        document: dict[str, object] = {
            "mode": self.object_store_mode,
            "bucket": self.object_store_bucket,
            "region": self.object_store_region,
        }
        if self.object_store_endpoint is not None:
            document["endpoint"] = self.object_store_endpoint
        if self.object_store_health_url is not None:
            document["healthUrl"] = self.object_store_health_url
        return document

    def _backup_document(self) -> dict[str, object]:
        document: dict[str, object] = {
            "repository": self.backup_repository,
            "retentionDays": self.retention_days,
            "rpoMinutes": self.rpo_minutes,
        }
        if self.provider_pitr_evidence_file is not None:
            document["providerPitrEvidenceFile"] = self.provider_pitr_evidence_file
        return document

    def _telemetry_document(self) -> dict[str, object]:
        if self.alert_webhook_url is None:
            return {"enabled": False}
        return {"enabled": True, "alertWebhookUrl": self.alert_webhook_url}


def write_new_config(config: SelfHostConfig, output: Path) -> None:
    """以仅新建语义写入配置，拒绝静默覆盖运营者文件。"""

    document = config.document()
    output.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    try:
        descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError as error:
        raise SelfHostConfigError(f"配置已存在，拒绝覆盖：{output}") from error
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        output.unlink(missing_ok=True)
        raise
