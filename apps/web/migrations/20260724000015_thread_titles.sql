WITH first_user_messages AS (
    SELECT DISTINCT ON (runs.task_id)
        runs.task_id,
        trim(regexp_replace(message_parts.message, '[[:space:]]+', ' ', 'g')) AS message
    FROM run_events
    JOIN runs ON runs.id = run_events.run_id
    CROSS JOIN LATERAL (
        SELECT string_agg(part.value->>'text', ' ' ORDER BY part.ordinality) AS message
        FROM jsonb_array_elements(
            COALESCE(run_events.payload->'data'->'content', '[]'::jsonb)
        ) WITH ORDINALITY AS part(value, ordinality)
        WHERE part.value->>'type' IN ('text', 'inputText')
          AND trim(COALESCE(part.value->>'text', '')) <> ''
    ) AS message_parts
    WHERE run_events.event_type = 'codex.item.completed'
      AND run_events.payload->>'itemType' = 'userMessage'
      AND message_parts.message IS NOT NULL
    ORDER BY runs.task_id, run_events.sequence ASC
),
normalized_titles AS (
    SELECT
        task_id,
        CASE
            WHEN char_length(message) > 80 THEN left(message, 79) || '…'
            ELSE message
        END AS title
    FROM first_user_messages
    WHERE message <> ''
)
UPDATE tasks
SET title = normalized_titles.title,
    updated_at = now()
FROM normalized_titles
WHERE tasks.id = normalized_titles.task_id
  AND tasks.title IN ('Thread', 'New Agent');
