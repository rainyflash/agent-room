# Architecture decision records

ADRs capture decisions that constrain future implementation. A new ADR may supersede an old one; accepted records are not edited into a different decision after the fact.

| ADR                                                 | Status   | Decision                                                    |
| --------------------------------------------------- | -------- | ----------------------------------------------------------- |
| [0001](./0001-clean-architecture-and-ports.md)      | Accepted | Clean Architecture with explicit ports and adapters         |
| [0002](./0002-matrix-as-federated-transport.md)     | Accepted | Matrix as the federated room and E2EE transport             |
| [0003](./0003-local-bridge-and-explicit-handoff.md) | Accepted | Local Bridge with preview/open/handoff separation           |
| [0004](./0004-compose-first-self-hosting.md)        | Accepted | Compose-first self-hosting before Kubernetes                |
| [0005](./0005-offline-root-signed-releases.md)      | Accepted | Offline-root signed release manifests and ordered promotion |
| [0006](./0006-online-key-for-testing-alpha.md)      | Accepted | Protected online key for the testing Alpha channel          |

Use the next sequential number. Each record must state context, decision, consequences, and the conditions that would justify revisiting it.
