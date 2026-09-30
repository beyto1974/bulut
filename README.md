# Bulut

A small dropbox for sessions. Each session has a short, readable link (`/k7m3q`) and holds files. People use
the web UI, scripts and language models use the REST API, `llms.txt`, `openapi.json` or MCP. Handy for passing
builds, logs and recordings between a development environment and a person, in either direction.

![A session with files, versions, tags and notes](docs/screenshots/session.png)

<p>
  <img src="docs/screenshots/session-dark.png" alt="The same page in dark mode" width="49%">
  <img src="docs/screenshots/qr.png" alt="The QR code of a session link" width="49%">
</p>

- **Short links.** Five lowercase letters and digits without look-alikes (no `0 o 1 l i`), so a code can be read
  aloud. A QR code of the link is one click away.
- **Big files.** Up to 1 GB, sent in resumable chunks. A dropped connection continues where it stopped.
- **Versions and tags.** Uploading a name that exists adds a new version. Tag versions (`v1.4.2`, `stable`);
  the newest one is always `latest`. Both the created time of the file and the upload time are kept.
- **Notes.** A description for the session and a note on every file.
- **Parallel uploads.** Files upload at the same time, so a small file never waits behind a large one.
- **Agent friendly.** REST, `llms.txt`, an OpenAPI description and an MCP endpoint.
- **Tidy.** A session is deleted, with its files, after 7 days without any activity (views, downloads and
  uploads all count). A session holds at most 100 files; new versions do not count.

## ⚠️ Put authentication in front of it

**Bulut has no authentication.** Anyone who can reach it can create sessions and read, upload and delete files.
A session code only identifies a session, it is not a secret.

Run it behind a reverse proxy (Traefik, nginx, Caddy, ...) that requires

- **basic authentication** for people using the web UI, and
- **a bearer token** for agents and REST clients (`Authorization: Bearer ...`).

Do not publish the port directly. The examples below bind to `127.0.0.1` for this reason. Configuring the proxy
is up to you and is not covered here.

## Quick start with Docker Compose

`examples/` has a complete stack: Bulut, Postgres and [Garage](https://garagehq.deuxfleurs.fr/) for storage.

```bash
git clone https://github.com/beyto1974/bulut.git
cd bulut/examples
./setup.sh                 # writes .env with fresh secrets, starts the stack, prepares the storage
```

Open http://localhost:8080. `HOST_PORT=9000 ./setup.sh` uses another port. To start over:
`docker compose down -v && rm .env`.

The setup needs `openssl` for the secrets and pulls `ghcr.io/beyto1974/bulut:latest`. Bulut starts only when
Postgres is healthy, creates its bucket on first start and applies its database migrations itself.

## Run the image

Bulut needs a Postgres database and an S3-compatible bucket (Garage, MinIO, AWS S3, ...). Everything is
configured through environment variables.

```bash
docker run -d --name bulut \
  -p 127.0.0.1:8080:8080 \
  --read-only --cap-drop ALL --security-opt no-new-privileges:true \
  -e DATABASE_URL='postgresql://bulut:PASSWORD@db.example.com:5432/bulut' \
  -e S3_ENDPOINT_URL='https://s3.example.com' \
  -e S3_BUCKET='bulut' \
  -e S3_ACCESS_KEY_ID='...' \
  -e S3_SECRET_ACCESS_KEY='...' \
  -e S3_REGION='us-east-1' \
  -e BASE_URL='https://bulut.example.com' \
  ghcr.io/beyto1974/bulut:latest
```

`BASE_URL` is how the app is reached from outside. Short links and QR codes use it. The image is about 8 MB,
has no shell, and runs as a non-root user. Images are tagged `latest`, `vX.Y.Z` and the short commit.

The container answers `GET /healthz` and has a built-in health check.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `DATABASE_URL` | required | Postgres connection URL. Migrations run at startup. |
| `S3_ENDPOINT_URL` | none | Endpoint of an S3-compatible store. Leave out for AWS S3. |
| `S3_BUCKET` | required | Bucket for the files. Created at startup if it does not exist. |
| `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | required | Credentials for the bucket. |
| `S3_REGION` | `us-east-1` | Region. Garage uses `garage`. |
| `BASE_URL` | `http://localhost:PORT` | Public address, used for links and QR codes. |
| `PORT` | `8080` | Port inside the container. |
| `APP_ENV` | `production` | `production`, `testing` or `development`. Anything but production is shown in the page title and header. |
| `LOG_LEVEL` | `info` | `error`, `warn`, `info`, `debug` or `trace`. Logs are JSON lines on stdout. |
| `MAX_FILES_PER_SESSION` | `100` | Most files in one session. New versions do not count. |
| `MAX_FILE_BYTES` | `1073741824` | Largest file, 1 GiB. |
| `CHUNK_SIZE` | `8388608` | Upload part size, at least 5 MiB. The file limit must fit in 10000 parts. |
| `SESSION_IDLE_TTL_DAYS` | `7` | Days without activity before a session and its files are deleted. |
| `SWEEP_INTERVAL_SECS` | `3600` | How often idle sessions are looked for. |
| `CODE_LENGTH` | `5` | Characters in a session code. |

`.env.example` lists them all with comments.

## Using it

**Web UI.** Open the address, press *New session*, drop files on the page. Share the link or the QR code.

**REST.** Every call is described in [`web/openapi.json`](web/openapi.json), which the app also serves at
`/openapi.json`. Add the `Authorization` header your proxy requires.

```bash
# create a session, then upload with one request
curl -X POST https://bulut.example.com/api/s
curl -T build.apk 'https://bulut.example.com/api/s/k7m3q/upload?name=build.apk&tag=v1.4.2'

# download the newest version, or a tagged one
curl -OJ 'https://bulut.example.com/api/s/k7m3q/download?name=build.apk'
curl -OJ 'https://bulut.example.com/api/s/k7m3q/download?name=build.apk&tag=v1.4.1'
```

Files larger than a few hundred MB are better sent with the chunked calls (`/uploads`), which can be resumed.

**Language models.** `GET /<code>/llms.txt` describes a session as plain text, `GET /llms.txt` explains the
service. `POST /mcp` is an MCP endpoint (streamable HTTP, JSON-RPC over POST) with tools to create a session,
read it, upload and read text files, set notes and tags.

## Development

You need Rust, a Postgres database and an S3-compatible bucket.

```bash
cp .env.example .env       # fill in DATABASE_URL and the S3_* values
cargo test                 # unit tests and integration tests against those services
cargo run
```

`src/domain` holds the types, `src/ports` the traits, `src/adapters` the Postgres and S3 implementations,
`src/services` the logic, `src/http` the routes, `src/llms.rs` the text indexes and `src/qr.rs` the QR code.
The UI is plain HTML, CSS and JavaScript in `web/`, embedded in the binary, with no build step.

End-to-end scripts run against a running instance (the browser test needs Playwright with Chromium):

```bash
node e2e/ui.test.js http://localhost:8080         # real browser: upload, versions, tags, notes, QR dialog
python3 e2e/big_upload.py http://localhost:8080   # 1 GB in chunks with a resume, hash-checked download
python3 e2e/parallel_upload.py http://localhost:8080
python3 e2e/file_limit.py http://localhost:8080
```

## CI and releases

`.github/workflows/ci.yml` runs on GitHub-hosted runners. Every push and pull request is checked for format,
lint and tests (Postgres and Garage as real services), then end to end in a real browser. The image is built
and reviewed on every run. A push to `main` bumps the patch version in `VERSION` (committed and tagged `vX.Y.Z`),
builds the image from that commit and pushes it to `ghcr.io/beyto1974/bulut`. An optional webhook
(`IMAGE_UPDATED_WEBHOOK_URL` secret) tells a server to pull it. `VERSION` is the single source of the
version: it is baked into the binary and shown in the UI footer and at `GET /api/version`.

Security policy and how to report a problem: [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE).
