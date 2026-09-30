-- Uploads in progress. The bytes live in S3 as a multipart upload, this row remembers where the
-- finished file goes so a client can resume after a dropped connection.
CREATE TABLE uploads (
    id                UUID PRIMARY KEY,
    session_code      TEXT        NOT NULL REFERENCES sessions (code) ON DELETE CASCADE,
    parent_id         UUID        REFERENCES nodes (id) ON DELETE CASCADE,
    name              TEXT        NOT NULL,
    content_type      TEXT        NOT NULL,
    size              BIGINT      NOT NULL CHECK (size >= 0),
    part_size         INTEGER     NOT NULL CHECK (part_size > 0),
    blob_key          TEXT        NOT NULL,
    s3_upload_id      TEXT        NOT NULL,
    client_created_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL
);
CREATE INDEX uploads_session_idx ON uploads (session_code);
CREATE INDEX uploads_created_idx ON uploads (created_at);
