# Todo

## Phases
- [x] 1. Scaffold: config, JSON logging, `/healthz`, `/api/version`, Dockerfile, compose, docs
- [x] 2. Domain: `ShortCode` and code generator
- [x] 3. Persistence: migrations, `SessionRepo` on Postgres, `last_activity_at`
- [x] 4. Sessions and tree API (`openapi.json` comes with phase 8)
- [x] 5. S3 blob store, chunked resumable uploads, range downloads
- [x] 6. Versions, tags, created and uploaded times
- [ ] 8. Agent access: `llms.txt`, MCP tools
- [ ] 9. Web UI (from the chosen mockup), env badge, version footer
- [ ] 10. QR code: `/qr.svg` and dialog
- [x] 11. Idle sweeper (7 days, from `.env`)
- [ ] 12. CI (mirror the tako CI config), auto version bump, final image review

## Later
- Thumbnails for images (the `thumb_key` column and `has_thumbnail` field already exist, the UI shows a file icon until then)
- Mail and OTP, themed HTML mails (not planned: access control is handled by Traefik)
- Per-file QR code
