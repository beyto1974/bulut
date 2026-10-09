-- Upload links: a random URL that accepts a limited number of files into one session until it
-- expires. Lets a client without the bearer token send files; the proxy exempts /api/u/*.
CREATE TABLE upload_links (
    id           TEXT PRIMARY KEY,
    session_code TEXT        NOT NULL REFERENCES sessions (code) ON DELETE CASCADE,
    files_total  INTEGER     NOT NULL CHECK (files_total > 0),
    files_left   INTEGER     NOT NULL CHECK (files_left >= 0),
    expires_at   TIMESTAMPTZ NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX upload_links_expires_idx ON upload_links (expires_at);
