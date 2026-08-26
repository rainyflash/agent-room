# Security Policy

## Supported versions

Agent Room has not published a production-supported release. The `main` branch receives security fixes, but it is not a support promise and must not be treated as a production release. A version support table will be added only when the first signed stable release passes the public Go/No-Go gate.

## Report a vulnerability privately

Use the repository's [private security advisory form](https://github.com/agent-room/agent-room/security/advisories/new). Do not open a public issue, discussion, pull request, or chat message containing exploit details, credentials, private agent content, or vulnerable deployment coordinates.

Include:

- affected commit or version and component;
- minimal reproduction steps or a proof of concept;
- impact, prerequisites, and whether federation or local access is required;
- any known workaround;
- a safe way to contact you through the advisory thread.

If private vulnerability reporting is unavailable, disclose only that the private channel is unavailable in a public issue. Do not include vulnerability details until a maintainer establishes a private channel.

## Response targets

- acknowledgement within 3 business days;
- initial severity and scope assessment within 7 business days;
- remediation plan or status update within 14 business days;
- coordinated publication after a fix and supported upgrade path exist.

These are response targets, not a paid support SLA. Complex federation, cryptography, or upstream issues may take longer; the advisory will receive status updates at least every 14 days.

## High-priority boundaries

Reports are especially valuable when they affect E2EE, device recovery, OIDC, Matrix federation, local IPC authentication, explicit content handoff, attachment integrity, secret storage, authorization, release signatures, downgrade protection, or deletion/export behavior.

Remote messages must not automatically enter an agent context, invoke a tool, or trigger a send. The Codex plugin must not read Codex private caches or hold Matrix device keys. A bypass of either boundary is a security issue.

## Safe research

Use accounts, devices, domains, and content you own or have explicit permission to test. Avoid privacy violations, denial of service, persistence, destructive data changes, and access beyond the minimum needed to demonstrate impact. Stop testing and report immediately if you encounter real user data or credentials.

Good-faith research that follows this policy will not be pursued by project maintainers. This statement cannot bind third parties or override local law.
