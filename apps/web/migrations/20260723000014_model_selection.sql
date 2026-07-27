-- Persist the model choice that belongs to each Thread/Task. The platform-wide
-- default remains in platform_configuration and is copied here when a new
-- Thread is created.

ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS model_provider TEXT,
    ADD COLUMN IF NOT EXISTS model TEXT;

ALTER TABLE tasks DROP CONSTRAINT IF EXISTS tasks_model_selection_pair_check;
ALTER TABLE tasks ADD CONSTRAINT tasks_model_selection_pair_check CHECK (
    (model_provider IS NULL AND model IS NULL)
    OR (
        model_provider IS NOT NULL
        AND btrim(model_provider) <> ''
        AND model IS NOT NULL
        AND btrim(model) <> ''
    )
);
