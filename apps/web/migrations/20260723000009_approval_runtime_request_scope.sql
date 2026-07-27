DROP INDEX IF EXISTS idx_approvals_runtime_request;

CREATE UNIQUE INDEX idx_approvals_runtime_request
    ON approvals(profile_id, thread_id, runtime_request_id)
    WHERE runtime_request_id IS NOT NULL AND thread_id IS NOT NULL;
