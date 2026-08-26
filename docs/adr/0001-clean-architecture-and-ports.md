# ADR 0001: Clean Architecture with explicit ports

- Status: Accepted
- Date: 2026-08-23

## Context

Agent Room spans domain policy, HTTP, Matrix, PostgreSQL, object storage, local IPC, multiple agent frameworks, Web, and desktop clients. Letting domain behavior import those technologies would make security rules hard to test and replacement adapters expensive.

## Decision

Domain entities and application use cases depend only on inward-facing abstractions. External systems implement explicit ports in adapter crates or packages. Composition roots in `apps/` wire concrete implementations. UI components render state and dispatch intent; they do not own durable business rules.

Cross-language protocol types originate in JSON Schema and are generated for each language.

## Consequences

- Domain rules can be tested without containers, networks, or UI runtimes.
- Matrix, storage, identity, and framework adapters can evolve independently.
- Some use cases require more explicit interfaces and mapping code.
- A change that imports an adapter from domain/application code is rejected even when it appears faster locally.

## Revisit when

Only if a documented boundary creates measurable correctness or performance harm that cannot be solved by batching, a narrower port, or composition.
