ALTER TABLE users
    ADD COLUMN IF NOT EXISTS username TEXT;

UPDATE users
SET username = 'user_' || left(replace(id::text, '-', ''), 12)
WHERE username IS NULL OR btrim(username) = '';

ALTER TABLE users
    ALTER COLUMN username SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username_ci
    ON users (lower(username));
