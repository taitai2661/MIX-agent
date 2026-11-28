-- Skills are keyed by name in the API (`get` / `delete` / `import` all look up
-- `json_extract(data, '$.name')`), but the table never enforced that. Two
-- concurrent imports could therefore create two rows with the same name and
-- only one was ever reachable. Enforce the invariant in SQLite instead of
-- relying on a SELECT-then-INSERT race window in `SkillRepository::save`.
--
-- The DELETE is safe to replay: after the first pass there is at most one row
-- per name, so it removes nothing. Keep the highest rowid (the most recently
-- inserted copy) when repairing an existing database.

DELETE FROM skills
WHERE rowid NOT IN (
    SELECT MAX(rowid) FROM skills GROUP BY json_extract(data, '$.name')
);

CREATE UNIQUE INDEX IF NOT EXISTS skills_name_unique_idx
ON skills(json_extract(data, '$.name'));
