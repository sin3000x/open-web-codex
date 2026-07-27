-- Centralized platform configuration. The first release stores browser-visible
-- settings globally. Scope columns make the later move to per-user settings a
-- data migration instead of a new storage system.

CREATE TABLE IF NOT EXISTS platform_configuration (
    scope_kind   TEXT NOT NULL
                     CHECK (scope_kind IN ('global', 'user')),
    scope_id     TEXT NOT NULL,
    config_key   TEXT NOT NULL,
    config_value JSONB NOT NULL,
    updated_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_kind, scope_id, config_key),
    CHECK (
        (scope_kind = 'global' AND scope_id = 'global')
        OR (scope_kind = 'user' AND scope_id <> '')
    )
);

CREATE INDEX IF NOT EXISTS idx_platform_configuration_scope
    ON platform_configuration(scope_kind, scope_id);
