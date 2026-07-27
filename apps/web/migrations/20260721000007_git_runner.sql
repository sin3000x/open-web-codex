-- Git workspace ownership and lease-based Runner state.

ALTER TABLE projects ADD COLUMN IF NOT EXISTS created_by UUID REFERENCES users(id) ON DELETE SET NULL;
UPDATE projects p
SET created_by = (
    SELECT m.user_id FROM memberships m WHERE m.organization_id = p.organization_id LIMIT 1
)
WHERE p.created_by IS NULL
  AND p.organization_id IS NOT NULL
  AND (SELECT COUNT(*) FROM memberships m WHERE m.organization_id = p.organization_id) = 1;

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS created_by UUID REFERENCES users(id) ON DELETE SET NULL;
UPDATE tasks t SET created_by = p.created_by
FROM projects p
WHERE t.project_id = p.id AND t.created_by IS NULL;

ALTER TABLE runs ADD COLUMN IF NOT EXISTS requested_by UUID REFERENCES users(id) ON DELETE SET NULL;
UPDATE runs r SET requested_by = t.created_by
FROM tasks t
WHERE r.task_id = t.id AND r.requested_by IS NULL;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS requested_profile_id UUID REFERENCES profiles(id) ON DELETE RESTRICT;
UPDATE runs r SET requested_profile_id = p.id
FROM profiles p
WHERE r.requested_by = p.owner_user_id
  AND r.organization_id = p.organization_id
  AND r.requested_profile_id IS NULL
  AND (SELECT COUNT(*) FROM profiles candidate
       WHERE candidate.owner_user_id = r.requested_by
         AND candidate.organization_id = r.organization_id
         AND candidate.status = 'active') = 1;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS source_ref TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS idempotency_key TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS lease_owner TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS heartbeat_at TIMESTAMPTZ;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS failure_code TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS active_turn_id TEXT;

ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS source_ref TEXT;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS head_commit TEXT;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS branch_name TEXT;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS retired_at TIMESTAMPTZ;
WITH duplicate_workspaces AS (
    SELECT id, row_number() OVER (
        PARTITION BY run_id ORDER BY created_at DESC, id DESC
    ) AS position
    FROM workspaces
    WHERE run_id IS NOT NULL
)
UPDATE workspaces SET run_id = NULL, updated_at = now()
WHERE id IN (SELECT id FROM duplicate_workspaces WHERE position > 1);
CREATE UNIQUE INDEX IF NOT EXISTS idx_workspaces_one_per_run
    ON workspaces(run_id) WHERE run_id IS NOT NULL;

ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE SET NULL;

ALTER TABLE runs DROP CONSTRAINT IF EXISTS runs_status_check;
ALTER TABLE runs ADD CONSTRAINT runs_status_check CHECK (
    status IN (
        'pending', 'provisioning', 'running', 'cancelling',
        'recovery_pending', 'completed', 'cancelled', 'failed'
    )
);
ALTER TABLE tasks DROP CONSTRAINT IF EXISTS tasks_status_check;
ALTER TABLE tasks ADD CONSTRAINT tasks_status_check CHECK (
    status IN ('pending', 'running', 'completed', 'cancelled', 'archived', 'failed')
);
ALTER TABLE workspaces DROP CONSTRAINT IF EXISTS workspaces_state_check;
ALTER TABLE workspaces ADD CONSTRAINT workspaces_state_check CHECK (
    state IN ('provisioning', 'ready', 'busy', 'cleanup_pending', 'failed', 'retired')
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_runs_idempotency
    ON runs(organization_id, requested_by, idempotency_key)
    WHERE requested_by IS NOT NULL AND idempotency_key IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_runs_scheduler
    ON runs(status, lease_expires_at, created_at)
    WHERE status IN ('pending', 'provisioning', 'running', 'cancelling', 'recovery_pending');
WITH duplicate_active_runs AS (
    SELECT id, row_number() OVER (
        PARTITION BY task_id ORDER BY created_at DESC, id DESC
    ) AS position
    FROM runs
    WHERE status IN ('pending', 'provisioning', 'running', 'cancelling', 'recovery_pending')
)
UPDATE runs
SET status = 'failed', failure_code = 'superseded_active_run',
    lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL,
    updated_at = now()
WHERE id IN (SELECT id FROM duplicate_active_runs WHERE position > 1);
CREATE UNIQUE INDEX IF NOT EXISTS idx_runs_one_active_per_task
    ON runs(task_id)
    WHERE status IN ('pending', 'provisioning', 'running', 'cancelling', 'recovery_pending');

CREATE TABLE IF NOT EXISTS runner_jobs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    run_id            UUID REFERENCES runs(id) ON DELETE CASCADE,
    workspace_id      UUID REFERENCES workspaces(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('workspace_cleanup')),
    state             TEXT NOT NULL DEFAULT 'pending'
                          CHECK (state IN ('pending', 'running', 'completed', 'failed')),
    attempt           INTEGER NOT NULL DEFAULT 0,
    run_after         TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner       TEXT,
    lease_token       TEXT,
    lease_expires_at  TIMESTAMPTZ,
    last_error_code   TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, kind)
);
CREATE INDEX IF NOT EXISTS idx_runner_jobs_pending
    ON runner_jobs(state, run_after, created_at)
    WHERE state IN ('pending', 'running');
