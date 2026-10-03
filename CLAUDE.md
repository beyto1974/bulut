# Bulut, guidance for Claude

Rust (axum) service, Postgres via sqlx, S3-compatible storage for files, plain-JS UI in `web/`.

## Rules
- TDD: write the failing test first. SOLID: depend on traits (`ports`), keep handlers thin, put logic in services.
- One commit per reasonable iteration.
- Code, comments and markdown are in English.
- Logs are JSON on stdout, level from `LOG_LEVEL`. Never log secrets or file contents.
- Keep the container image small and free of secrets: multi-stage build, no shell in the runtime image, non-root.
- After each change: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`.
- Version lives in `VERSION` only.

## Releases
Do not edit `VERSION` by hand: CI bumps the patch number on every push to `main` (`scripts/bump-version.sh`) and
builds the image from the bumped commit. Commits by CI carry `[skip ci]`.

## Decisions
- No authentication in the app, by design: no accounts, no PINs, no tokens. The deployment is protected by a
  reverse proxy (bearer token for agents and REST, basic auth for humans). Do not add auth code to the app.
- Idle expiry: `sessions.last_activity_at` is touched on any access (throttled to once per minute per session).
  A sweeper deletes sessions idle longer than `SESSION_IDLE_TTL_DAYS`, including their stored files.
- Limits per session (files, versions per file, bytes, unfinished uploads, tags) are enforced in the repository
  under a per-session lock, so parallel uploads cannot overshoot. Keep new limits there.
- Folders are implemented but disabled by default (`FOLDERS_ENABLED`). Do not build UI for them or mention them in
  user-facing docs until that is planned.
- Uploaded files are served as downloads with a sandboxing CSP. The only exception is `?inline=1` for PNG, JPEG, GIF,
  WebP and AVIF, and only when the first bytes match the stored type (`inline_image` in `src/http/files.rs`). Never
  inline SVG, HTML, PDF or anything else, and do not add other previews on the application's origin.
- Same name creates a new file version. The `latest` tag moves to the newest upload.
- Mail, OTP and themed HTML mails are out of scope.
