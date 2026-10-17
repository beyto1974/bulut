-- A per-session log of what changed, so clients can follow a session instead of listing it again.
-- `seq` is handed out under the session row lock, so it is gapless and follows commit order.
ALTER TABLE sessions ADD COLUMN change_seq BIGINT NOT NULL DEFAULT 0;

CREATE TABLE changes (
    session_code TEXT        NOT NULL REFERENCES sessions (code) ON DELETE CASCADE,
    seq          BIGINT      NOT NULL,
    kind         TEXT        NOT NULL,
    node_id      UUID,
    version_id   UUID,
    name         TEXT,
    version      INTEGER,
    tag          TEXT,
    at           TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (session_code, seq)
);
