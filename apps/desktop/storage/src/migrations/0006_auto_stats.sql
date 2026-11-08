-- Auto routing statistics (schema version 6).
--
-- The scorer needs per-model, per-profile performance to stop treating every
-- candidate as equally fast and equally available. Raw per-turn rows would be
-- read on every Auto resolution, so this table keeps one EWMA row per key
-- instead. `profile = '*'` is the aggregate used when a profile-specific row
-- is still too young to trust.
--
-- `last_failure_kind` stores the normalized taxonomy from
-- `mixd_providers::reliability::ProviderErrorKind`; the older
-- `auto_reliability_events` table still carries the raw event for audit.

CREATE TABLE IF NOT EXISTS auto_model_stats (
    model_id          TEXT NOT NULL,
    profile           TEXT NOT NULL,
    success_count     INTEGER NOT NULL DEFAULT 0,
    failure_count     INTEGER NOT NULL DEFAULT 0,
    ewma_ttft_ms      REAL,
    ewma_output_tps   REAL,
    ewma_duration_ms  REAL,
    last_failure_kind TEXT,
    last_failure_at   TEXT,
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (model_id, profile)
);

CREATE INDEX IF NOT EXISTS auto_model_stats_profile_idx
    ON auto_model_stats(profile, updated_at);

-- Feedback is read per owner and bounded by age. Without this the resolver
-- scanned every rating ever written. The owner leads the index because the
-- query always filters it, then `created_at` bounds the range.
DROP INDEX IF EXISTS message_feedback_created_idx;
CREATE INDEX IF NOT EXISTS message_feedback_owner_created_idx
    ON message_feedback(owner_id, created_at);
CREATE INDEX IF NOT EXISTS message_feedback_profile_idx
    ON message_feedback(json_extract(data, '$.profile'));
