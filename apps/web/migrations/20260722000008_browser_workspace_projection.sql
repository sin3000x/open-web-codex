-- Browser workspace projections for durable worktree/clone entries.

ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_kind TEXT NOT NULL DEFAULT 'main';
ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_name TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_parent_run_id UUID REFERENCES runs(id) ON DELETE SET NULL;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_group_run_id UUID REFERENCES runs(id) ON DELETE SET NULL;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS workspace_copy_agents_md BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS fork_thread_id TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS fork_source_run_id UUID REFERENCES runs(id) ON DELETE SET NULL;

ALTER TABLE runs DROP CONSTRAINT IF EXISTS runs_workspace_kind_check;
ALTER TABLE runs ADD CONSTRAINT runs_workspace_kind_check
    CHECK (workspace_kind IN ('main', 'worktree', 'clone'));

CREATE INDEX IF NOT EXISTS idx_runs_browser_workspaces
    ON runs(organization_id, workspace_kind, updated_at DESC)
    WHERE workspace_kind <> 'main';

CREATE INDEX IF NOT EXISTS idx_runs_workspace_parent
    ON runs(workspace_parent_run_id)
    WHERE workspace_parent_run_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_runs_workspace_group
    ON runs(workspace_group_run_id, updated_at DESC)
    WHERE workspace_group_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS terminal_sessions (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id     UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    run_id              UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    browser_workspace_id UUID NOT NULL,
    terminal_id         TEXT NOT NULL,
    process_id          TEXT NOT NULL UNIQUE,
    state               TEXT NOT NULL DEFAULT 'starting'
                            CHECK (state IN ('starting', 'running', 'closing', 'closed', 'failed')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (organization_id, run_id, terminal_id)
);

CREATE INDEX IF NOT EXISTS idx_terminal_sessions_active
    ON terminal_sessions(organization_id, run_id, updated_at DESC)
    WHERE state IN ('starting', 'running', 'closing');

-- Per-user browser presentation and safe workspace automation state. The key
-- is either a Project id (main workspace) or a Run id (derived workspace).
CREATE TABLE IF NOT EXISTS browser_workspace_preferences (
    organization_id      UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id              UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    browser_workspace_id UUID NOT NULL,
    settings             JSONB NOT NULL DEFAULT '{}'::jsonb,
    runtime_codex_args   TEXT,
    setup_completed_script TEXT,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, user_id, browser_workspace_id)
);

CREATE INDEX IF NOT EXISTS idx_browser_workspace_preferences_user
    ON browser_workspace_preferences(organization_id, user_id, updated_at DESC);
