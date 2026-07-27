-- Authorized, lazy MCP Resource cache for structured reply-card data.

CREATE TABLE reply_artifacts (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    run_id             UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    thread_id          TEXT NOT NULL,
    turn_id            TEXT NOT NULL,
    producer_item_id   TEXT NOT NULL,
    source_server       TEXT NOT NULL,
    source_uri          TEXT NOT NULL,
    mime_type           TEXT,
    expected_size       BIGINT,
    content             BYTEA,
    content_sha256      TEXT,
    state               TEXT NOT NULL DEFAULT 'pending'
                            CHECK (state IN ('pending', 'ready', 'failed')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, thread_id, source_server, source_uri)
);

CREATE INDEX idx_reply_artifacts_run
    ON reply_artifacts(run_id, created_at);
