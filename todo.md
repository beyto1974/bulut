# Todo

## Phases
- [x] 1. Scaffold: config, JSON logging, `/healthz`, `/api/version`, Dockerfile, compose, docs
- [x] 2. Domain: `ShortCode` and code generator
- [x] 3. Persistence: migrations, `SessionRepo` on Postgres, `last_activity_at`
- [ ] 4. Sessions and tree API, `openapi.json`
- [ ] 5. S3 blob store, chunked resumable uploads, range downloads
- [ ] 6. Versions, tags, created and uploaded times
- [ ] 7. Thumbnails
- [ ] 8. Agent access: `llms.txt`, MCP tools
- [ ] 9. Web UI (from the chosen mockup), env badge, version footer
- [ ] 10. QR code: `/qr.svg` and dialog
- [ ] 11. Idle sweeper (7 days, from `.env`)
- [ ] 12. CI (mirror the tako CI config), auto version bump, final image review

## Later
- Mail and OTP, themed HTML mails
- Per-file QR code
