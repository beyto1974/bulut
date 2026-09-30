CREATE TABLE sessions (
    code             TEXT PRIMARY KEY,
    description      TEXT        NOT NULL DEFAULT '',
    pin_hash         TEXT,
    created_at       TIMESTAMPTZ NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_last_activity_idx ON sessions (last_activity_at);

CREATE TABLE nodes (
    id           UUID PRIMARY KEY,
    session_code TEXT        NOT NULL REFERENCES sessions (code) ON DELETE CASCADE,
    parent_id    UUID        REFERENCES nodes (id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL CHECK (kind IN ('file', 'folder')),
    name         TEXT        NOT NULL CHECK (length(name) BETWEEN 1 AND 255 AND position('/' IN name) = 0),
    note         TEXT        NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL
);
-- A name is unique inside one folder; NULL parent (session root) needs a sentinel.
CREATE UNIQUE INDEX nodes_unique_name_idx
    ON nodes (session_code, COALESCE(parent_id, '00000000-0000-0000-0000-000000000000'::uuid), name);
CREATE INDEX nodes_parent_idx ON nodes (parent_id);

CREATE TABLE file_versions (
    id                UUID PRIMARY KEY,
    node_id           UUID        NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    version           INTEGER     NOT NULL,
    size              BIGINT      NOT NULL CHECK (size >= 0),
    content_type      TEXT        NOT NULL,
    sha256            TEXT,
    blob_key          TEXT        NOT NULL,
    thumb_key         TEXT,
    client_created_at TIMESTAMPTZ,
    uploaded_at       TIMESTAMPTZ NOT NULL,
    is_latest         BOOLEAN     NOT NULL DEFAULT TRUE,
    UNIQUE (node_id, version)
);
CREATE UNIQUE INDEX file_versions_latest_idx ON file_versions (node_id) WHERE is_latest;

CREATE TABLE tags (
    node_id    UUID NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    version_id UUID NOT NULL REFERENCES file_versions (id) ON DELETE CASCADE,
    tag        TEXT NOT NULL CHECK (length(tag) BETWEEN 1 AND 64),
    -- A tag points to one version of a file at a time; tagging a newer version moves it.
    PRIMARY KEY (node_id, tag)
);
CREATE INDEX tags_version_idx ON tags (version_id);
