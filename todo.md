# Roadmap

Ideas and known limits, roughly in order of usefulness.

## Known limits
- `llms.txt` and the MCP `get_session` tool run one query per file (up to 1000 files). Batch the versions into one
  query.
- If the database fails right after the object store finished a chunked upload, `complete` discards the stored
  file and the client has to send it again. Keep the upload row so `complete` can be retried.
- Files of a session that is purged while an upload into it completes can stay in the bucket. A periodic
  comparison of the bucket with the database (or a lifecycle rule) would clean them up.
- There is no limit on the number of sessions. Put request rate limits and timeouts on the reverse proxy.

## Later
- Folders. The backend exists (tree, folder creation, uploads into a folder, breadcrumbs) and is switched off with
  `FOLDERS_ENABLED=false`. Still to do: folder navigation and creation in the UI, folder notes, `llms.txt` and MCP
  wording, then switch the default on.
- Thumbnails for images (the `thumb_key` column and `has_thumbnail` field already exist, the UI shows a file icon).
- A QR code per file.
