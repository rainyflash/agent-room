"""把生产领域配置渲染成各组件的确定性运行文件。"""

from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
from typing import Final
from urllib.parse import quote

from .config import DeploymentConfig
from .secrets import SecretStore


GENERATED_FILE_MODE: Final = 0o600
PUBLIC_FILE_MODE: Final = 0o644
OIDC_MAPPING_SOURCE: Final = (
    Path(__file__).resolve().parents[2] / "infra" / "synapse" / "agent_room_oidc_mapping.py"
)


@dataclass(frozen=True, slots=True)
class DeploymentPaths:
    state: Path
    generated: Path
    data: Path
    secrets: Path
    compose_environment: Path
    worker_override: Path

    @classmethod
    def from_state(cls, state: Path) -> "DeploymentPaths":
        resolved = state.expanduser().resolve()
        if not resolved.is_absolute() or resolved == Path(resolved.anchor):
            raise ValueError("生产状态目录必须是非根绝对路径。")
        return cls(
            state=resolved,
            generated=resolved / "generated",
            data=resolved / "data",
            secrets=resolved / "secrets",
            compose_environment=resolved / "compose.env",
            worker_override=resolved / "generated" / "compose.workers.json",
        )

    def prepare(self) -> None:
        for directory in (
            self.state,
            self.generated,
            self.data,
            self.secrets,
            self.generated / "caddy",
            self.generated / "keycloak",
            self.generated / "seaweedfs",
            self.generated / "synapse" / "workers",
            self.data / "caddy-data",
            self.data / "caddy-config",
            self.data / "clamav",
            self.data / "object-store",
            self.data / "postgres",
            self.data / "synapse",
        ):
            directory.mkdir(mode=0o700, parents=True, exist_ok=True)


def render_deployment(
    config: DeploymentConfig,
    paths: DeploymentPaths,
    secrets: SecretStore,
) -> None:
    paths.prepare()
    migration_url = _postgres_url(
        config.database.control_migration_user,
        secrets.read("agent_room_db_migration_password"),
        config.database.host,
        config.database.port,
        config.database.control_database,
        config.database.tls_mode,
    )
    secrets.write_derived("migration_database_url", migration_url)
    _write_text(
        paths.compose_environment,
        _compose_environment(config, paths, secrets),
        PUBLIC_FILE_MODE,
    )
    _write_json(
        paths.generated / "keycloak" / "realm-agent-room.json",
        _keycloak_realm(config, secrets),
        GENERATED_FILE_MODE,
    )
    _write_json(
        paths.generated / "seaweedfs" / "s3.json",
        _seaweed_config(secrets),
        GENERATED_FILE_MODE,
    )
    _write_text(
        paths.generated / "synapse" / "agent-room-appservice.yaml",
        _synapse_appservice(config, secrets),
        GENERATED_FILE_MODE,
    )
    _write_text(
        paths.generated / "synapse" / "homeserver.yaml",
        _synapse_homeserver(config, secrets),
        GENERATED_FILE_MODE,
    )
    _write_text(
        paths.generated / "synapse" / "log.config",
        _synapse_log_config(),
        PUBLIC_FILE_MODE,
    )
    _write_text(
        paths.generated / "synapse" / "agent_room_oidc_mapping.py",
        OIDC_MAPPING_SOURCE.read_text(encoding="utf-8"),
        PUBLIC_FILE_MODE,
    )
    _render_workers(config, paths)
    _write_text(
        paths.generated / "caddy" / "Caddyfile",
        _caddyfile(config),
        PUBLIC_FILE_MODE,
    )


def _compose_environment(
    config: DeploymentConfig,
    paths: DeploymentPaths,
    secrets: SecretStore,
) -> str:
    database = config.database
    public = config.public
    object_store = config.object_store
    profiles = [*config.compose_profiles]
    if config.telemetry_enabled:
        profiles.append("telemetry")
    if config.capacity.synapse_workers > 0:
        profiles.append("workers")
    values = {
        "AGENT_ROOM_STATE_DIR": paths.state.as_posix(),
        "AGENT_ROOM_PROJECT_NAME": config.project_name,
        "AGENT_ROOM_SERVER_NAME": public.server_name,
        "AGENT_ROOM_APP_DOMAIN": public.app_domain,
        "AGENT_ROOM_API_DOMAIN": public.api_domain,
        "AGENT_ROOM_MATRIX_DOMAIN": public.matrix_domain,
        "AGENT_ROOM_IDENTITY_DOMAIN": public.identity_domain,
        "AGENT_ROOM_ACME_EMAIL": public.acme_email,
        "AGENT_ROOM_DB_HOST": database.host,
        "AGENT_ROOM_DB_PORT": str(database.port),
        "AGENT_ROOM_DB_TLS_MODE": database.tls_mode,
        "AGENT_ROOM_DB_NAME": database.control_database,
        "AGENT_ROOM_DB_MIGRATION_USER": database.control_migration_user,
        "AGENT_ROOM_DB_RUNTIME_USER": database.control_runtime_user,
        "SYNAPSE_DB_NAME": database.synapse_database,
        "SYNAPSE_DB_USER": database.synapse_user,
        "KEYCLOAK_DB_NAME": database.identity_database,
        "KEYCLOAK_DB_USER": database.identity_user,
        "AGENT_ROOM_CONTENT_S3_ENDPOINT": object_store.endpoint,
        "AGENT_ROOM_OBJECT_STORE_HEALTH_URL": object_store.health_url,
        "AGENT_ROOM_CONTENT_S3_BUCKET": object_store.bucket,
        "AGENT_ROOM_CONTENT_S3_REGION": object_store.region,
        "AGENT_ROOM_CONTENT_S3_CREATE_BUCKET": (
            "true" if object_store.mode == "embedded" else "false"
        ),
        "AGENT_ROOM_CONTENT_MATRIX_AGENT_ID": secrets.read("content_matrix_agent_id"),
        "AGENT_ROOM_CONTROL_PLANE_REPLICAS": str(config.capacity.control_plane_replicas),
        "AGENT_ROOM_SYNAPSE_WORKERS": str(config.capacity.synapse_workers),
        "AGENT_ROOM_OTLP_TRACES_ENDPOINT": (
            "http://telemetry:4318/v1/traces" if config.telemetry_enabled else ""
        ),
        "COMPOSE_PROFILES": ",".join(profiles),
    }
    return "".join(f"{name}={value}\n" for name, value in values.items())


def _keycloak_realm(config: DeploymentConfig, secrets: SecretStore) -> dict[str, object]:
    public = config.public
    return {
        "realm": "agent-room",
        "enabled": True,
        "displayName": "Agent Room",
        "registrationAllowed": False,
        "loginWithEmailAllowed": True,
        "sslRequired": "external",
        "bruteForceProtected": True,
        "clients": [
            {
                "clientId": "agent-room-web",
                "name": "Agent Room Web",
                "enabled": True,
                "publicClient": False,
                "secret": secrets.read("keycloak_web_client_secret"),
                "standardFlowEnabled": True,
                "directAccessGrantsEnabled": False,
                "redirectUris": [f"{public.api_origin}/auth/oidc/callback"],
                "webOrigins": [public.app_origin],
                "attributes": {"pkce.code.challenge.method": "S256"},
            },
            {
                "clientId": "agent-room-bridge",
                "name": "Agent Room Bridge",
                "enabled": True,
                "publicClient": True,
                "protocol": "openid-connect",
                "standardFlowEnabled": False,
                "implicitFlowEnabled": False,
                "directAccessGrantsEnabled": False,
                "serviceAccountsEnabled": False,
                "attributes": {"oauth2.device.authorization.grant.enabled": "true"},
            },
            {
                "clientId": "agent-room-matrix",
                "name": "Agent Room Matrix",
                "enabled": True,
                "publicClient": False,
                "secret": secrets.read("keycloak_matrix_client_secret"),
                "protocol": "openid-connect",
                "standardFlowEnabled": True,
                "implicitFlowEnabled": False,
                "directAccessGrantsEnabled": False,
                "serviceAccountsEnabled": False,
                "redirectUris": [f"{public.matrix_origin}/_synapse/client/oidc/callback"],
                "webOrigins": [],
            },
        ],
    }


def _seaweed_config(secrets: SecretStore) -> dict[str, object]:
    return {
        "identities": [
            {
                "name": "agent-room-control-plane",
                "credentials": [
                    {
                        "accessKey": secrets.read("s3_access_key"),
                        "secretKey": secrets.read("s3_secret_key"),
                    }
                ],
                "actions": ["Admin", "Read", "Write", "List", "Tagging"],
            }
        ]
    }


def _synapse_appservice(config: DeploymentConfig, secrets: SecretStore) -> str:
    server_pattern = re.escape(config.public.server_name)
    return f'''id: "agent-room"
url: null
as_token: {_yaml(secrets.read("synapse_appservice_token"))}
hs_token: {_yaml(secrets.read("synapse_appservice_hs_token"))}
sender_localpart: "_agent_room"
namespaces:
  users:
    - exclusive: true
      regex: '^@_agent_[0-9a-f]{{32}}:{server_pattern}$'
  aliases:
    - exclusive: true
      regex: '^#agent-room-[a-z0-9][a-z0-9._=-]{{0,254}}:{server_pattern}$'
  rooms: []
rate_limited: false
receive_ephemeral: false
'''


def _synapse_homeserver(config: DeploymentConfig, secrets: SecretStore) -> str:
    public = config.public
    database = config.database
    worker_lines = ""
    if config.capacity.synapse_workers > 0:
        worker_lines = f'''  - port: 9093
    bind_addresses: ['0.0.0.0']
    type: http
    resources:
      - names: [replication]
redis:
  enabled: true
  host: redis
  port: 6379
worker_replication_secret: {_yaml(secrets.read("worker_replication_secret"))}
instance_map:
  main:
    host: synapse
    port: 9093
'''
    return f'''server_name: {_yaml(public.server_name)}
public_baseurl: {_yaml(public.matrix_origin + "/")}
pid_file: /data/homeserver.pid
app_service_config_files:
  - /config/agent-room-appservice.yaml
listeners:
  - port: 8008
    bind_addresses: ['0.0.0.0']
    tls: false
    type: http
    x_forwarded: true
    resources:
      - names: [client, federation]
        compress: false
  - port: 9001
    bind_addresses: ['0.0.0.0']
    type: metrics
{worker_lines}database:
  name: psycopg2
  args:
    user: {_yaml(database.synapse_user)}
    password: {_yaml(secrets.read("synapse_db_password"))}
    database: {_yaml(database.synapse_database)}
    host: {_yaml(database.host)}
    port: {database.port}
    cp_min: 2
    cp_max: 20
log_config: /config/log.config
media_store_path: /data/media_store
registration_shared_secret: {_yaml(secrets.read("synapse_registration_secret"))}
report_stats: false
enable_metrics: true
macaroon_secret_key: {_yaml(secrets.read("synapse_macaroon_secret"))}
form_secret: {_yaml(secrets.read("synapse_form_secret"))}
signing_key_path: /data/{public.server_name}.signing.key
trusted_key_servers:
  - server_name: "matrix.org"
suppress_key_server_warning: true
enable_registration: false
oidc_providers:
  - idp_id: agent_room
    idp_name: "Agent Room"
    discover: false
    issuer: {_yaml(public.identity_origin + "/realms/agent-room")}
    client_id: "agent-room-matrix"
    client_secret: {_yaml(secrets.read("keycloak_matrix_client_secret"))}
    authorization_endpoint: {_yaml(public.identity_origin + "/realms/agent-room/protocol/openid-connect/auth")}
    token_endpoint: "http://identity:8080/realms/agent-room/protocol/openid-connect/token"
    userinfo_endpoint: "http://identity:8080/realms/agent-room/protocol/openid-connect/userinfo"
    jwks_uri: "http://identity:8080/realms/agent-room/protocol/openid-connect/certs"
    scopes: ["openid", "profile", "email"]
    enable_registration: true
    user_mapping_provider:
      module: "agent_room_oidc_mapping.AgentRoomOidcMappingProvider"
      config:
        issuer: {_yaml(public.identity_origin + "/realms/agent-room")}
rc_login:
  address:
    per_second: 5
    burst_count: 20
  account:
    per_second: 2
    burst_count: 10
  failed_attempts:
    per_second: 1
    burst_count: 5
retention:
  enabled: true
  default_policy:
    max_lifetime: 30d
  allowed_lifetime_min: 1d
  allowed_lifetime_max: 3650d
  purge_jobs:
    - longest_max_lifetime: 3650d
      interval: 1d
'''


def _synapse_log_config() -> str:
    return '''version: 1
formatters:
  structured:
    format: '%(asctime)s %(levelname)s %(name)s %(message)s'
handlers:
  console:
    class: logging.StreamHandler
    formatter: structured
    stream: ext://sys.stdout
root:
  level: INFO
  handlers: [console]
disable_existing_loggers: false
'''


def _render_workers(config: DeploymentConfig, paths: DeploymentPaths) -> None:
    services: dict[str, object] = {}
    upstreams: list[str] = []
    for index in range(1, config.capacity.synapse_workers + 1):
        name = f"synapse-worker-{index}"
        upstreams.append(f"{name}:8083")
        _write_text(
            paths.generated / "synapse" / "workers" / f"worker-{index}.yaml",
            f'''worker_app: synapse.app.generic_worker
worker_name: worker_{index}
worker_listeners:
  - type: http
    port: 8083
    bind_addresses: ['0.0.0.0']
    x_forwarded: true
    resources:
      - names: [client, federation]
worker_log_config: /config/log.config
''',
            GENERATED_FILE_MODE,
        )
        services[name] = {
            "image": "matrixdotorg/synapse:v1.159.0",
            "entrypoint": ["python", "-m", "synapse.app.generic_worker"],
            "command": [
                "--config-path",
                "/config/homeserver.yaml",
                "--config-path",
                f"/config/workers/worker-{index}.yaml",
            ],
            "depends_on": {
                "redis": {"condition": "service_healthy"},
                "synapse": {"condition": "service_healthy"},
            },
            "environment": {"PYTHONPATH": "/config"},
            "healthcheck": {
                "test": [
                    "CMD-SHELL",
                    "python -c \"import urllib.request; urllib.request.urlopen('http://127.0.0.1:8083/_matrix/client/versions', timeout=3)\"",
                ],
                "interval": "10s",
                "timeout": "5s",
                "retries": 30,
            },
            "networks": ["backend"],
            "read_only": True,
            "restart": "unless-stopped",
            "security_opt": ["no-new-privileges:true"],
            "tmpfs": ["/tmp:rw,noexec,nosuid,size=64m", "/run:rw,noexec,nosuid,size=16m"],
            "volumes": [
                "${AGENT_ROOM_STATE_DIR}/generated/synapse:/config:ro",
                "${AGENT_ROOM_STATE_DIR}/data/synapse:/data",
            ],
        }
    override = {"services": services} if services else {"services": {}}
    _write_json(paths.worker_override, override, PUBLIC_FILE_MODE)
    _write_text(
        paths.generated / "caddy" / "worker-upstreams.txt",
        " ".join(upstreams) + ("\n" if upstreams else ""),
        PUBLIC_FILE_MODE,
    )


def _caddyfile(config: DeploymentConfig) -> str:
    public = config.public
    worker_route = ""
    if config.capacity.synapse_workers > 0:
        upstreams = " ".join(
            f"synapse-worker-{index}:8083"
            for index in range(1, config.capacity.synapse_workers + 1)
        )
        worker_route = f'''\t@sync path_regexp sync ^/_matrix/client/(v3|r0|unstable)/(sync|events|initialSync)$
\treverse_proxy @sync {upstreams}
'''
    csp = (
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; "
        "style-src-attr 'unsafe-inline'; img-src 'self' data: blob: mxc:; "
        "font-src 'self' data:; "
        f"connect-src 'self' {public.api_origin} {public.matrix_origin} "
        f"wss://{public.matrix_domain}; media-src 'self' blob:; worker-src 'self' blob:; "
        "manifest-src 'self'; object-src 'none'; frame-src 'none'; base-uri 'none'; "
        "form-action 'self'; frame-ancestors 'none'; upgrade-insecure-requests"
    )
    return f'''{{
\temail {public.acme_email}
\tadmin off
}}

{public.server_name} {{
\theader Content-Type application/json
\trespond /.well-known/matrix/server `{{"m.server":"{public.matrix_domain}:443"}}` 200
\trespond /.well-known/matrix/client `{{"m.homeserver":{{"base_url":"{public.matrix_origin}"}}}}` 200
\trespond 404
}}

{public.app_domain} {{
\tencode zstd gzip
\troot * /srv/web
\ttry_files {{path}} /index.html
\theader {{
\t\tContent-Security-Policy "{csp}"
\t\tCross-Origin-Opener-Policy "same-origin"
\t\tPermissions-Policy "camera=(), microphone=(), geolocation=(), payment=(), usb=()"
\t\tReferrer-Policy "no-referrer"
\t\tX-Content-Type-Options "nosniff"
\t}}
\trespond /_agent-room/healthz 200
\tfile_server
}}

{public.api_domain} {{
\trequest_body {{
\t\tmax_size 32MB
\t}}
\treverse_proxy control-plane:8090
}}

{public.matrix_domain} {{
\trequest_body {{
\t\tmax_size 64MB
\t}}
{worker_route}\treverse_proxy synapse:8008
}}

{public.identity_domain} {{
\treverse_proxy identity:8080
}}
'''


def _postgres_url(
    username: str,
    password: str,
    host: str,
    port: int,
    database: str,
    tls_mode: str,
) -> str:
    return (
        f"postgresql://{quote(username, safe='')}:{quote(password, safe='')}"
        f"@{host}:{port}/{quote(database, safe='')}?sslmode={quote(tls_mode, safe='')}"
    )


def _yaml(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def _write_json(path: Path, value: object, mode: int) -> None:
    _write_text(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n", mode)


def _write_text(path: Path, content: str, mode: int) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(content, encoding="utf-8", newline="\n")
    temporary.chmod(mode)
    os.replace(temporary, path)
    path.chmod(mode)
