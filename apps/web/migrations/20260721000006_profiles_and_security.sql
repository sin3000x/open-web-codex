-- Profile ownership, Workspace authorization, encrypted Secrets and durable
-- approval/audit facts. Runtime paths stay server-side and are never public DTOs.

ALTER TABLE projects ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
UPDATE projects
SET organization_id = (SELECT id FROM organizations ORDER BY created_at LIMIT 1)
WHERE organization_id IS NULL
  AND (SELECT COUNT(*) FROM organizations) = 1;
CREATE INDEX IF NOT EXISTS idx_projects_organization ON projects(organization_id);

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
UPDATE tasks
SET organization_id = projects.organization_id
FROM projects
WHERE tasks.project_id = projects.id AND tasks.organization_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_organization ON tasks(organization_id);

ALTER TABLE runs ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
UPDATE runs
SET organization_id = tasks.organization_id
FROM tasks
WHERE runs.task_id = tasks.id AND runs.organization_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_runs_organization ON runs(organization_id);

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
UPDATE sessions s
SET organization_id = (
    SELECT m.organization_id FROM memberships m WHERE m.user_id = s.user_id LIMIT 1
)
WHERE s.organization_id IS NULL
  AND (SELECT COUNT(*) FROM memberships m WHERE m.user_id = s.user_id) = 1;
CREATE INDEX IF NOT EXISTS idx_sessions_organization ON sessions(organization_id);

CREATE TABLE IF NOT EXISTS profiles (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    owner_user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    runtime_key       TEXT NOT NULL UNIQUE,
    name              TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'active'
                          CHECK (status IN ('active', 'disabled', 'error')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (organization_id, owner_user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_profiles_owner ON profiles(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_profiles_organization ON profiles(organization_id);

CREATE TABLE IF NOT EXISTS profile_capabilities (
    profile_id         UUID PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    server_build       TEXT NOT NULL,
    protocol_version   TEXT NOT NULL,
    manifest           JSONB NOT NULL,
    observed_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS profile_secrets (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    profile_id        UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    provider_id       TEXT NOT NULL,
    purpose           TEXT NOT NULL DEFAULT 'provider_api_key',
    key_version       TEXT NOT NULL,
    nonce             BYTEA NOT NULL,
    ciphertext        BYTEA NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (profile_id, provider_id, purpose)
);
CREATE INDEX IF NOT EXISTS idx_profile_secrets_profile ON profile_secrets(profile_id);

CREATE TABLE IF NOT EXISTS workspaces (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    project_id        UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id        UUID NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
    run_id            UUID REFERENCES runs(id) ON DELETE SET NULL,
    root_path         TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'ready'
                          CHECK (state IN ('provisioning', 'ready', 'busy', 'failed', 'retired')),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (profile_id, root_path)
);
CREATE INDEX IF NOT EXISTS idx_workspaces_project ON workspaces(project_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_run ON workspaces(run_id);

ALTER TABLE approvals ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS profile_id UUID REFERENCES profiles(id) ON DELETE CASCADE;
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS thread_id TEXT;
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS runtime_request_id TEXT;
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending', 'dispatching', 'delivery_unknown', 'approved', 'rejected', 'expired', 'cancelled'));
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX IF NOT EXISTS idx_approvals_runtime_request
    ON approvals(profile_id, runtime_request_id)
    WHERE runtime_request_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_approvals_pending
    ON approvals(organization_id, state, created_at)
    WHERE state = 'pending';

ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS request_id TEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS outcome TEXT NOT NULL DEFAULT 'success';
CREATE INDEX IF NOT EXISTS idx_audit_log_organization_created
    ON audit_log(organization_id, created_at DESC);
