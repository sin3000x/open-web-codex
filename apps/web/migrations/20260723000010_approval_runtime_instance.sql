ALTER TABLE approvals ADD COLUMN IF NOT EXISTS runtime_instance_id UUID;

UPDATE approvals
SET state = 'cancelled', decided_at = COALESCE(decided_at, now()), version = version + 1
WHERE runtime_instance_id IS NULL
  AND state IN ('pending', 'dispatching', 'delivery_unknown');

DROP INDEX IF EXISTS idx_approvals_runtime_request;

CREATE UNIQUE INDEX idx_approvals_runtime_request
    ON approvals(profile_id, runtime_instance_id, runtime_request_id)
    WHERE runtime_instance_id IS NOT NULL AND runtime_request_id IS NOT NULL;
