# ADR 0004: Compose-first self-hosting

- Status: Accepted
- Date: 2026-08-26

## Context

Early public operators need a reproducible single-host deployment and a credible path to external databases, object storage, control-plane replicas, and Synapse workers. Kubernetes would add an orchestration product before measured scheduling pressure exists.

## Decision

Ship a strict Docker Compose production reference with generated configuration, mounted secrets, startup validation, health/federation diagnostics, backups, restore drills, and observable failure modes. Scale through configuration and external service adapters first. Do not add Kubernetes until load and operations evidence demonstrates a multi-host scheduling requirement.

## Consequences

- A small operator can deploy without editing internal databases.
- The reference topology has a clear single-host failure domain.
- External services remain operator responsibilities with explicit least-privilege contracts.
- Multi-region or multi-host scheduling is outside the current support boundary.

## Revisit when

Sustained measurements show that the accepted capacity or availability target requires automated multi-host scheduling and cannot be met by the documented externalization path.
