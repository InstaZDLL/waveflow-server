# Security Policy

Thank you for reporting security vulnerabilities responsibly before any public
disclosure.

## Supported Versions

`waveflow-server` does not have long-term version support yet. The main branch
and the latest published release should be considered the only supported
versions for security fixes.

| Version                                           | Supported |
| ------------------------------------------------- | --------- |
| `main` / latest published release                 | Yes       |
| Older versions, snapshots, and unmaintained forks | No        |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Use one of these channels, depending on what is available on the public
repository:

1. **GitHub Security Advisories**: open the repository's _Security_ tab, then
   choose _Report a vulnerability_. This is the recommended confidential
   channel.
2. Contact the maintainers privately if GitHub Security Advisories are not
   available.

Your report should include:

- a clear description of the issue;
- the expected impact on users or their data;
- the affected version, deployment shape (self-hosted, container, …), and
  Postgres version;
- the affected HTTP endpoints, sync operations, JWT verification path, or
  storage code;
- detailed reproduction steps;
- a minimal proof of concept if needed to understand the issue;
- a suggested mitigation if you have one;
- your contact information for follow-up.

## Response Targets

- Initial acknowledgement: within 3 business days.
- First assessment: within 7 business days.
- Follow-up updates: at least once per week until the fix is released.
- Public disclosure: coordinated after the fix is released, usually within 30
  to 90 days depending on severity and impact.

## Current Scope

The most sensitive `waveflow-server` surfaces are:

- the REST API exposed under `/api/v1/*` and the audit trail it builds;
- JWT verification against the JWKS endpoint Better Auth publishes;
- the sync-operations log and the WebSocket fan-out it powers;
- the streaming endpoint (`/stream/<id>`), HTTP range handling, and the
  transcode cache;
- the Postgres pool, migration runner, and the `waveflow-core::repository`
  implementations the handlers delegate to;
- the plugin host (RFC-002) once it lands here in Phase 3.f.

## Out of Scope

The following reports are generally not considered exploitable security
vulnerabilities:

- cosmetic bugs or interface issues without security impact;
- generic automated reports without a demonstrated exploit path;
- vulnerabilities in unmodified third-party services (Postgres, axum, sqlx);
- denial-of-service that requires admin-level access on the self-hosted box;
- voluntary disclosure of files or secrets by the user.

## Rewards

WaveFlow does not offer a monetary bug bounty. Researchers who report a valid
vulnerability may be credited in the fix release notes if they want attribution.
