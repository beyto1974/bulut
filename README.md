# Bulut

A small dropbox for sessions. Each session has a short, human-readable URL and holds files and folders.
People use the web UI, LLM agents use REST, `llms.txt`, `openapi.json` or the MCP tool.

- Short code: 5 lowercase letters and digits, without look-alike characters (`0 o 1 l i`). Length is set by `CODE_LENGTH`.
- Files can be up to 1 GB, uploaded in resumable chunks (S3 multipart).
- Files and folders can have a note. A session has a description.
- Uploading a file with an existing name adds a new version. Versions can be tagged (`v1.4.2`, `latest`).
  Both the created time and the uploaded time are kept.
- Every session has a QR code of its link.
- A session is deleted after `SESSION_IDLE_TTL_DAYS` (default 7) days without any activity: views, downloads,
  uploads and API reads all count as activity.

## Configuration

Copy `.env.example` to `.env`. Settings are read from the environment.
`APP_ENV` other than `production` is shown in the UI header and page title. `LOG_LEVEL` sets the JSON log level.

## Development

```bash
cargo test
devdb create --project bulut        # private database on the shared dev Postgres
devgarage create --project bulut    # private bucket on the shared dev Garage
HOST_PORT=$(freeport) docker compose up -d --no-deps backend
```

Open `http://localhost:<HOST_PORT>`. Do not start a per-environment `db` or `garage`.

## Layout

`src/config.rs` settings, `src/logging.rs` JSON logging, `src/http.rs` routes. Domain, ports, adapters and services
are added per phase, see `todo.md`. The UI is plain HTML, CSS and JS in `web/`, no build step.

## Versioning

The `VERSION` file is the single source. CI bumps it on each release, the binary embeds it, and the UI footer
and `GET /api/version` show it.
