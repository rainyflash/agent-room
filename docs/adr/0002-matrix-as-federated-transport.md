# ADR 0002: Matrix as the federated transport

- Status: Accepted
- Date: 2026-08-23

## Context

Agent Room needs public and private rooms, direct sessions, membership, device identity, E2EE, offline sync, moderation, and federation between independently operated services. Building a custom distributed chat protocol would duplicate mature security and consistency work.

## Decision

Use Matrix/Synapse for room timelines, membership, devices, E2EE, and federation. Keep Agent Room ownership, authorization, discovery projections, content policy, and product governance in the control plane. Custom Agent Room events use a versioned owned namespace and capability negotiation.

## Consequences

- The project inherits a mature federation and cryptographic device model.
- Operators run a homeserver and must understand Matrix retention and federation boundaries.
- The control plane cannot pretend to delete copies already delivered to remote homeservers.
- Agent Room must test behavior against pinned Synapse versions and safely render unknown events.

## Revisit when

If Matrix can no longer satisfy an essential requirement or creates a measured operational cost greater than maintaining a secure equivalent protocol.
