# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue. The contact is GitHub itself, there is
no email address: open
[a private security advisory](https://github.com/beyto1974/bulut/security/advisories/new) (repository
**Security** tab, then **Report a vulnerability**). Only the maintainer can read it.
Include what you found, how to reproduce it, and the version (`GET /api/version`).

You will get an answer through the advisory within a few days. Fixes are released as a new patch version of
the image, and the advisory is published once it is out.

## Supported versions

Only the latest release (`ghcr.io/beyto1974/bulut:latest`) receives fixes.

## What Bulut does not do

**Bulut has no authentication.** Anyone who can reach it can create sessions and read, upload and delete
files. It is meant to run behind a reverse proxy that requires a bearer token (for agents and REST clients)
or basic authentication (for people). Do not expose it to the internet without one. A short session code
only identifies a session, it is not a secret.

Because of this, these are not vulnerabilities in Bulut: reaching an instance that has no proxy in front of
it, guessing a session code, or reading the files of a session whose code you know.

## Hardening already in place

- The container runs as a numeric non-root user with no shell, and the compose files drop all capabilities
  and use a read-only root filesystem.
- Uploaded files are served as downloads with `nosniff` and a sandboxing `Content-Security-Policy`, never
  rendered on the application's origin. The UI has a strict CSP without inline scripts or styles.
- Names, tags and sizes are validated, and file counts and sizes are limited per session.
- Secrets are read from the environment and never logged.
