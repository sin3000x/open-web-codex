-- Inline visualization Artifacts belong to a Run/Thread. The producing Turn is
-- provenance only and must not prevent a later Turn in the same Thread from
-- embedding the Artifact.

ALTER TABLE inline_visualization_artifacts
    RENAME COLUMN turn_id TO producer_turn_id;

COMMENT ON COLUMN inline_visualization_artifacts.producer_turn_id IS
    'Turn that produced the Artifact; provenance only, not an authorization boundary';
