-- Encrypted counterpart to platform_configuration for server-only credentials.
-- The initial release uses global scope; user scope is reserved for the
-- multi-user storage migration.

CREATE TABLE IF NOT EXISTS platform_configuration_secrets (
    scope_kind   TEXT NOT NULL
                     CHECK (scope_kind IN ('global', 'user')),
    scope_id     TEXT NOT NULL,
    config_key   TEXT NOT NULL,
    key_version  TEXT NOT NULL,
    nonce        BYTEA NOT NULL,
    ciphertext   BYTEA NOT NULL,
    updated_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_kind, scope_id, config_key),
    CHECK (
        (scope_kind = 'global' AND scope_id = 'global')
        OR (scope_kind = 'user' AND scope_id <> '')
    )
);

CREATE INDEX IF NOT EXISTS idx_platform_configuration_secrets_scope
    ON platform_configuration_secrets(scope_kind, scope_id);
