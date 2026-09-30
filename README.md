# Bulut

A small dropbox for sessions. Each session has a short, human-readable URL and holds files.
People use the web UI, LLM agents use REST, `llms.txt`, `openapi.json` or the MCP tool.

- Short code: 5 lowercase letters and digits, without look-alike characters (`0 o 1 l i`). Length is set by `CODE_LENGTH`.
- Files can be up to 1 GB, uploaded in resumable chunks (S3 multipart). A session holds at most 100 files
  (`MAX_FILES_PER_SESSION`). New versions of an existing file do not count.
- Files can have a note. A session has a description.
- Uploading a file with an existing name adds a new version. Versions can be tagged (`v1.4.2`, `latest`).
  Both the created time and the uploaded time are kept.
- Every session has a QR code of its link.
- Thumbnails are not generated yet, the UI shows a file icon.
- A session is deleted after `SESSION_IDLE_TTL_DAYS` (default 7) days without any activity: views, downloads,
  uploads and API reads all count as activity.

## Access control

The app has no authentication of its own. It is protected at the Traefik level: a bearer token for agents and
REST clients, and basic auth for humans using the web UI. The short code only identifies a session, it is not a secret.

## Configuration

Copy `.env.example` to `.env`. Settings are read from the environment.
`APP_ENV` other than `production` is shown in the UI header and page title. `LOG_LEVEL` sets the JSON log level.

## Development

```bash
cargo test
devdb create --project bulut        # private database on the shared dev Postgres
devgarage create --project bulut    # private bucket on the shared dev Garage
freeport                              # put the number in .env as HOST_PORT, and use it in BASE_URL
docker compose up -d --no-deps backend
```

Open `http://localhost:<HOST_PORT>`. Do not start a per-environment `db` or `garage`.

## Tests

`cargo test` runs unit tests and integration tests against the shared dev Postgres and Garage (settings from `.env`).
Two end-to-end scripts run against a running instance (the browser test needs Playwright with Chromium; if it is
installed globally, run it with `NODE_PATH=$(npm root -g)`):

```bash
node e2e/ui.test.js http://localhost:<port>         # real browser: upload, versions, tags, notes, QR dialog
python3 e2e/big_upload.py http://localhost:<port>   # 1 GB in chunks with a resume, hash-checked download
```

## Agents

- `GET /<code>/llms.txt` describes a session as text, `GET /llms.txt` explains the service.
- `GET /openapi.json` is the API description.
- `POST /mcp` is an MCP endpoint (streamable HTTP, JSON-RPC over POST) with tools to create a session, read it,
  upload and read text files, set notes and tags.

## Layout

`src/domain` types, `src/ports` traits, `src/adapters` Postgres and S3 implementations, `src/services` the logic,
`src/http` routes and handlers, `src/llms.rs` text indexes, `src/qr.rs` QR code. The UI is plain HTML, CSS and JS in
`web/` (embedded in the binary, no build step).

## Versioning and CI

The `VERSION` file is the single source. The binary embeds it, and the UI footer and `GET /api/version` show it.

`.github/workflows/ci.yml` follows the tako workflow (self-hosted runner, skip pushes already covered by a PR):

- every push and PR: `cargo fmt --check`, clippy, the tests (Postgres and an S3 mock as service containers),
  the bump script tests, and a build of the image that is reviewed for size and leaked settings.
- a push to `main`: the patch version is bumped and committed (`chore(release): vX.Y.Z [skip ci]`, tagged `vX.Y.Z`),
  the image is built from that commit and pushed as `latest`, `X.Y.Z` and the short commit, and an optional webhook
  is called.

Repository settings the workflow reads: variable `REGISTRY_URL`, secrets `REGISTRY_USERNAME`, `REGISTRY_PASSWORD`
and optionally `IMAGE_UPDATED_WEBHOOK_URL`. Without `REGISTRY_URL` nothing is published.

The running environment (`APP_ENV`) is a runtime setting, so one image serves production, testing and development.
