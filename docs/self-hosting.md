# Self-hosting Agent Room

This guide covers the Compose-first production reference. It does not convert the current public-beta No-Go into production support; review [known limitations](./known-limitations.md) before exposing a service.

## Host requirements

- dedicated x86-64 Linux host;
- Docker Engine and Compose 2.20+;
- minimum 4 GiB RAM and 20 GiB free disk; 8 GiB and 100 GiB are recommended;
- public TCP 80 and 443;
- Python 3.11+ and a checkout of the exact Agent Room release being deployed;
- a backup destination in a different failure domain.

The memory preflight treats up to 256 MiB as firmware or kernel reservation, so a genuine 4 GiB cloud instance is accepted even when Linux reports slightly less usable RAM. Hosts below 3.75 GiB visible RAM are still rejected.

The installer rejects non-Linux production installation, insufficient minimum memory/disk, unresolved DNS, occupied public ports, invalid configuration, or unhealthy dependencies.

## DNS

For the example base domain `room.example.com`, point all records below at the host before installation:

| Purpose                           | Example record            |
| --------------------------------- | ------------------------- |
| Matrix server name and delegation | `room.example.com`        |
| Web application                   | `app.room.example.com`    |
| Control API                       | `api.room.example.com`    |
| Matrix client/federation endpoint | `matrix.room.example.com` |
| OIDC identity provider            | `id.room.example.com`     |

The `example.com` records are reserved documentation values and will not work. Use domains you control. Public access to 80/443 is required for ACME and federation validation.

## Generate a configuration

The guided generator emits no credential and refuses to overwrite an existing file:

```bash
sudo install -d -m 0750 /etc/agent-room /var/lib/agent-room
sudo python3 tools/self_host.py init \
  --domain room.example.com \
  --output /etc/agent-room/deployment.json
```

The default profile uses embedded PostgreSQL and embedded S3-compatible object storage. It derives distinct service domains, stores backups at `/var/backups/agent-room`, retains them for 30 days, targets a 15-minute RPO, and leaves outbound paging disabled until an HTTPS alert receiver is explicitly supplied.

The ACME contact email is optional. Supply `--email operator@example.com` only if certificate-authority notifications are desired; omitting it does not disable Caddy automatic HTTPS or renewal.

Run the strict host, DNS, and port check:

```bash
sudo python3 tools/self_host.py doctor \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

## Install and verify

```bash
sudo python3 tools/self_host.py install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/self_host.py health \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/self_host.py federation \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

Installation creates stable random secrets, generates the Synapse signing key, renders configuration, validates Compose, builds/pulls images, applies migrations through a dedicated migration role, initializes the object bucket, starts the stack, checks public health, and validates Matrix delegation. Operators never edit application tables or Synapse's database directly.

Treat `/var/lib/agent-room` as irreplaceable state. Secret files are normalized to read-only `0444` inside a `0700` parent because non-Swarm Compose bind-mounts them directly into non-root containers. Generated container configuration follows the same boundary: its top-level directory stays `0700`, while mounted service directories and files are normalized to `0555` and `0444`. Host users still cannot traverse either private parent. Secrets must not enter Git, tickets, logs, or chat.

## Backups and restore drills

Install and verify the systemd backup timer after the service is healthy:

```bash
sudo python3 tools/self_host.py backup-schedule-install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/self_host.py backup-schedule-verify \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

Create and verify an on-demand backup:

```bash
sudo python3 tools/self_host.py backup \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/self_host.py backup-verify \
  --backup-id BACKUP_ID \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

Use `restore-drill` with the same arguments to prove an isolated recovery. A backup stored only on the application host is not disaster recovery.

## Upgrade and stop

Back up and verify before every upgrade. Check out a signed compatible release, review its migration and rollback notes, then run:

```bash
sudo python3 tools/self_host.py upgrade \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

The release order is database expansion, compatible server, clients, observation, then legacy-path contraction. Do not skip phases or mix arbitrary Bridge, plugin, desktop, and server versions.

To stop containers without deleting state:

```bash
sudo python3 tools/self_host.py stop \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

## External PostgreSQL and object storage

The generator exposes explicit `--database-mode external` and `--object-store-mode external` options. External PostgreSQL requires secure TLS, three application databases with the documented least-privilege roles, and a recent provider PITR evidence file. External object storage requires a pre-created bucket and separate credentials written to the generated secret files after configuration rendering.

Use [`infra/production/deployment.external.example.json`](../infra/production/deployment.external.example.json) only as a schema example. It contains reserved domains and fake contact data. The detailed role, worker, observability, and external-service contracts remain in [`infra/production/README.md`](../infra/production/README.md).

## Operational truth

- Grafana and Prometheus bind to loopback by default; access them through an authenticated administrative tunnel.
- Enabling telemetry requires a credential-free HTTPS alert webhook URL. A separate Bearer secret is generated locally.
- Public messages default to 30-day retention. Federation may leave copies governed by remote servers after local deletion.
- A clean public-host acceptance run has not yet completed. Follow the [Task 40 evidence](../specs/agent-room-foundation/task-40-validation.md) rather than assuming a rendered Compose file is production proof.
