-- Authorized typed visualization Artifacts referenced from Assistant messages.

CREATE TABLE inline_visualization_artifacts (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id     UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    run_id              UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    thread_id           TEXT NOT NULL,
    turn_id             TEXT NOT NULL,
    producer_item_id    TEXT NOT NULL,
    artifact_ref        TEXT NOT NULL,
    renderer_kind       TEXT NOT NULL,
    renderer_payload    JSONB NOT NULL,
    state               TEXT NOT NULL DEFAULT 'ready'
                            CHECK (state IN ('ready', 'failed')),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, artifact_ref)
);

CREATE INDEX idx_inline_visualization_artifacts_thread
    ON inline_visualization_artifacts(run_id, thread_id, created_at);
