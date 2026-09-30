# Bulut, guidance for Claude

Rust (axum) service, Postgres via sqlx, S3 (Garage) for blobs, plain-JS UI in `web/`.

## Rules
- TDD: write the failing test first. SOLID: depend on traits (`ports`), keep handlers thin, put logic in services.
- One commit per reasonable iteration. Do not add assistant attribution trailers to commits (a hook blocks them).
- Code, comments and markdown are in English.
- Logs are JSON on stdout, level from `LOG_LEVEL`. Never log secrets, PINs or full file contents.
- Ports: use `freeport`. Backing services: shared Postgres and Garage via `devdb` / `devgarage`, start with
  `docker compose up -d --no-deps backend`. Report URLs as `http://localhost:<port>`.
- After each phase: review the diff, check the image (`docker history`, size, no secrets in layers), update `todo.md`.
- Version lives in `VERSION` only.

## Commands
`cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `docker compose build`.

## Decisions
- Access is the link itself (no accounts). Optional per-session PIN.
- Idle expiry: `sessions.last_activity_at` is touched on any access (throttled to once per minute per session).
  A sweeper deletes sessions idle longer than `SESSION_IDLE_TTL_DAYS`, including blobs.
- Same name in a folder creates a new file version. The `latest` tag moves to the newest upload.
- Mail, OTP and themed HTML mails are out of scope for v1.
