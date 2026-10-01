# Roadmap

Ideas and known limits, roughly in order of usefulness.

## Known limits
- The orphan sweep (`ORPHAN_GRACE_HOURS`) lists finished objects only. An unfinished multipart upload that has no
  row in the database (the process died right after starting it) is not seen; a lifecycle rule on the bucket that
  aborts incomplete multipart uploads after a day covers it.
- There is no limit on the number of sessions. Put request rate limits and timeouts on the reverse proxy.

## Later
- Folders. The backend exists (tree, folder creation, uploads into a folder, breadcrumbs) and is switched off with
  `FOLDERS_ENABLED=false`. Still to do: folder navigation and creation in the UI, folder notes, `llms.txt` and MCP
  wording, then switch the default on.
- Thumbnails for images (the `thumb_key` column and `has_thumbnail` field already exist, the UI shows a file icon).
- A QR code per file.
