# Security Policy

Report vulnerabilities privately through GitHub Security Advisories. Do not open a public issue containing secrets, credentials or an exploit.

## Supported versions

Only `main` and the latest published release receive security fixes.

## Useful report details

- affected WaveFlow Server version and operating system;
- deployment shape and reverse proxy, if any;
- affected endpoint, CLI command or SQLite operation;
- minimal reproduction and expected impact;
- whether the instance key, database, audio root or a user credential is exposed.

## Current sensitive surfaces

- local Argon2id login and rotating opaque sessions;
- the SQLite database, migration runner and global writer coordinator;
- `instance.key` and encrypted per-user Subsonic credentials;
- library membership, and the catalogue and media authorization enforced
  inside the queries rather than in the handlers;
- URL/query redaction in request traces;
- canonical filesystem and symlink guards on the scanner and on streaming;
- AEAD-sealed stream tickets and public share tokens, both redacted from traces.

Cosmetic issues, generic scanner output without an exploit, administrator-triggered resource exhaustion on the administrator's own host and vulnerabilities in unmodified third-party software are normally out of scope.

WaveFlow does not currently offer a monetary bug bounty. Valid reporters may be credited in release notes after a coordinated fix.
