-- 0001_initial.sql
-- Initial MIX-agent Desktop schema. Mirrors the Docker edition's Alembic
-- `0001_initial` baseline, adapted to SQLite. JSON columns are stored as TEXT
-- containing JSON; FTS5 is used for memory search in lieu of pg_trgm.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS providers_owner_idx ON providers(owner_id);

CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS models_owner_idx ON models(owner_id);

CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS secrets_owner_idx ON secrets(owner_id);

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS agents_owner_idx ON agents(owner_id);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS workspaces_owner_idx ON workspaces(owner_id);

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS conversations_owner_idx ON conversations(owner_id);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS projects_owner_idx ON projects(owner_id);

CREATE TABLE IF NOT EXISTS conversation_folders (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS conversation_folders_owner_idx ON conversation_folders(owner_id);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    conversation_id TEXT NOT NULL REFERENCES conversations(id)
);
CREATE INDEX IF NOT EXISTS messages_owner_idx ON messages(owner_id);
CREATE INDEX IF NOT EXISTS messages_conversation_idx ON messages(conversation_id);
CREATE INDEX IF NOT EXISTS messages_conv_created_idx ON messages(conversation_id, created_at);

CREATE TABLE IF NOT EXISTS message_feedback (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    message_id TEXT UNIQUE NOT NULL REFERENCES messages(id)
);
CREATE INDEX IF NOT EXISTS message_feedback_message_idx ON message_feedback(message_id);

CREATE TABLE IF NOT EXISTS auto_reliability_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS auto_reliability_owner_created_idx ON auto_reliability_events(owner_id, created_at);

CREATE TABLE IF NOT EXISTS performance_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS performance_event_owner_created_idx ON performance_events(owner_id, created_at);

CREATE TABLE IF NOT EXISTS model_usage_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS model_usage_owner_created_idx ON model_usage_events(owner_id, created_at);

CREATE TABLE IF NOT EXISTS scheduled_jobs (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS scheduled_job_owner_enabled_idx ON scheduled_jobs(owner_id, created_at);

CREATE TABLE IF NOT EXISTS scheduled_runs (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    job_id TEXT NOT NULL REFERENCES scheduled_jobs(id),
    scheduled_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    run_id TEXT REFERENCES runs(id)
);
CREATE UNIQUE INDEX IF NOT EXISTS scheduled_runs_job_at_uq ON scheduled_runs(job_id, scheduled_at);
CREATE INDEX IF NOT EXISTS scheduled_runs_job_idx ON scheduled_runs(job_id);
CREATE INDEX IF NOT EXISTS scheduled_runs_scheduled_idx ON scheduled_runs(scheduled_at);
CREATE INDEX IF NOT EXISTS scheduled_runs_status_idx ON scheduled_runs(status);
CREATE INDEX IF NOT EXISTS scheduled_runs_run_idx ON scheduled_runs(run_id);

CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    read_at TEXT
);
CREATE INDEX IF NOT EXISTS notifications_owner_read_idx ON notifications(owner_id, read_at);

CREATE TABLE IF NOT EXISTS app_settings (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS app_settings_owner_idx ON app_settings(owner_id);

CREATE TABLE IF NOT EXISTS tools (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS tools_owner_idx ON tools(owner_id);

CREATE TABLE IF NOT EXISTS mcp_connections (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS mcp_connections_owner_idx ON mcp_connections(owner_id);

CREATE TABLE IF NOT EXISTS mcp_auth_states (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    state_hash TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS mcp_auth_state_hash_idx ON mcp_auth_states(state_hash);
CREATE INDEX IF NOT EXISTS mcp_auth_state_owner_created_idx ON mcp_auth_states(owner_id, created_at);

CREATE TABLE IF NOT EXISTS permission_rules (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS permission_rules_owner_idx ON permission_rules(owner_id);

CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    lifecycle_state TEXT NOT NULL DEFAULT 'established',
    role TEXT NOT NULL DEFAULT 'fact',
    scope TEXT NOT NULL DEFAULT 'user',
    source_kind TEXT NOT NULL DEFAULT 'agent',
    confidence REAL NOT NULL DEFAULT 0.7,
    salience REAL NOT NULL DEFAULT 0.5,
    strength REAL NOT NULL DEFAULT 0.6,
    activation_count INTEGER NOT NULL DEFAULT 0,
    last_activated_at TEXT,
    last_reinforced_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    task_id TEXT,
    superseded_by_id TEXT,
    disputed_by_id TEXT,
    verification TEXT NOT NULL DEFAULT 'unverified'
);
CREATE INDEX IF NOT EXISTS memory_owner_state_created_idx ON memories(owner_id, lifecycle_state, created_at);
CREATE INDEX IF NOT EXISTS memory_owner_role_scope_idx ON memories(owner_id, role, scope);
CREATE INDEX IF NOT EXISTS memory_owner_task_idx ON memories(owner_id, task_id);
CREATE INDEX IF NOT EXISTS memory_lifecycle_idx ON memories(lifecycle_state);
CREATE INDEX IF NOT EXISTS memory_role_idx ON memories(role);
CREATE INDEX IF NOT EXISTS memory_scope_idx ON memories(scope);
CREATE INDEX IF NOT EXISTS memory_source_kind_idx ON memories(source_kind);
CREATE INDEX IF NOT EXISTS memory_verification_idx ON memories(verification);

CREATE TABLE IF NOT EXISTS memory_associations (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    source_memory_id TEXT NOT NULL REFERENCES memories(id),
    target_memory_id TEXT NOT NULL REFERENCES memories(id),
    weight REAL NOT NULL DEFAULT 0.2,
    confidence REAL NOT NULL DEFAULT 0.5,
    coactivation_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS memory_assoc_owner_pair_uq ON memory_associations(owner_id, source_memory_id, target_memory_id);
CREATE INDEX IF NOT EXISTS memory_assoc_source_weight_idx ON memory_associations(owner_id, source_memory_id, weight);
CREATE INDEX IF NOT EXISTS memory_assoc_source_idx ON memory_associations(source_memory_id);
CREATE INDEX IF NOT EXISTS memory_assoc_target_idx ON memory_associations(target_memory_id);

CREATE TABLE IF NOT EXISTS memory_features (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    memory_id TEXT NOT NULL REFERENCES memories(id),
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    weight REAL NOT NULL DEFAULT 1.0
);
CREATE UNIQUE INDEX IF NOT EXISTS memory_feature_triplet_uq ON memory_features(memory_id, kind, value);
CREATE INDEX IF NOT EXISTS memory_feature_lookup_idx ON memory_features(owner_id, kind, value);
CREATE INDEX IF NOT EXISTS memory_feature_memory_idx ON memory_features(memory_id);

CREATE TABLE IF NOT EXISTS memory_processing_jobs (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    run_id TEXT NOT NULL REFERENCES runs(id),
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS memory_processing_jobs_run_uq ON memory_processing_jobs(run_id);
CREATE INDEX IF NOT EXISTS memory_processing_jobs_status_created_idx ON memory_processing_jobs(status, created_at);
CREATE INDEX IF NOT EXISTS memory_processing_jobs_run_idx ON memory_processing_jobs(run_id);

CREATE TABLE IF NOT EXISTS memory_action_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    run_id TEXT REFERENCES runs(id),
    candidate_index INTEGER NOT NULL DEFAULT 0,
    action_version INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS memory_action_run_candidate_uq ON memory_action_events(run_id, candidate_index, action_version);
CREATE INDEX IF NOT EXISTS memory_action_owner_created_idx ON memory_action_events(owner_id, created_at);
CREATE INDEX IF NOT EXISTS memory_action_run_idx ON memory_action_events(run_id);

CREATE TABLE IF NOT EXISTS memory_revisions (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS memory_revisions_owner_idx ON memory_revisions(owner_id);

CREATE TABLE IF NOT EXISTS memory_evidences (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    kind TEXT NOT NULL,
    ref TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    memory_id TEXT NOT NULL,
    captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS memory_evidence_owner_memory_idx ON memory_evidences(owner_id, memory_id);
CREATE INDEX IF NOT EXISTS memory_evidence_owner_kind_idx ON memory_evidences(owner_id, kind);
CREATE INDEX IF NOT EXISTS memory_evidence_memory_idx ON memory_evidences(memory_id);
CREATE INDEX IF NOT EXISTS memory_evidence_kind_idx ON memory_evidences(kind);

CREATE TABLE IF NOT EXISTS memory_conflicts (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    memory_a_id TEXT NOT NULL,
    memory_b_id TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    resolved INTEGER NOT NULL DEFAULT 0,
    resolution TEXT NOT NULL DEFAULT 'unresolved',
    resolved_at TEXT
);
CREATE INDEX IF NOT EXISTS memory_conflict_owner_resolved_idx ON memory_conflicts(owner_id, resolved, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS memory_conflict_pair_uq ON memory_conflicts(owner_id, memory_a_id, memory_b_id);
CREATE INDEX IF NOT EXISTS memory_conflict_a_idx ON memory_conflicts(memory_a_id);
CREATE INDEX IF NOT EXISTS memory_conflict_b_idx ON memory_conflicts(memory_b_id);
CREATE INDEX IF NOT EXISTS memory_conflict_resolution_idx ON memory_conflicts(resolution);

CREATE TABLE IF NOT EXISTS skills (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS skills_owner_idx ON skills(owner_id);

CREATE TABLE IF NOT EXISTS skill_revisions (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS skill_revisions_owner_idx ON skill_revisions(owner_id);

CREATE TABLE IF NOT EXISTS knowledge (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS knowledge_owner_doc_idx ON knowledge(owner_id, created_at);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS artifacts_owner_idx ON artifacts(owner_id);

CREATE TABLE IF NOT EXISTS audit_events (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS audit_events_owner_idx ON audit_events(owner_id);

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    status TEXT NOT NULL DEFAULT 'queued',
    request_key TEXT NOT NULL UNIQUE,
    CHECK (status IN ('queued','running','waiting_approval','paused','budget_extension_pending','completed','failed','cancelled','interrupted'))
);
CREATE INDEX IF NOT EXISTS runs_owner_idx ON runs(owner_id);
CREATE INDEX IF NOT EXISTS runs_conversation_idx ON runs(conversation_id);
CREATE INDEX IF NOT EXISTS runs_status_idx ON runs(status);
CREATE UNIQUE INDEX IF NOT EXISTS runs_request_key_uq ON runs(request_key);

CREATE TABLE IF NOT EXISTS run_checkpoints (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    run_id TEXT NOT NULL REFERENCES runs(id),
    step INTEGER NOT NULL DEFAULT 0,
    tool_count INTEGER NOT NULL DEFAULT 0,
    trigger TEXT NOT NULL DEFAULT 'interval'
);
CREATE INDEX IF NOT EXISTS run_checkpoint_run_step_idx ON run_checkpoints(run_id, step);
CREATE INDEX IF NOT EXISTS run_checkpoint_run_created_idx ON run_checkpoints(run_id, created_at);

CREATE TABLE IF NOT EXISTS run_events (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS run_events_run_seq_uq ON run_events(run_id, sequence);
CREATE INDEX IF NOT EXISTS run_events_run_idx ON run_events(run_id);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    run_id TEXT NOT NULL REFERENCES runs(id),
    status TEXT NOT NULL DEFAULT 'pending'
);
CREATE INDEX IF NOT EXISTS tool_calls_run_idx ON tool_calls(run_id);

CREATE TABLE IF NOT EXISTS approvals (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    run_id TEXT NOT NULL REFERENCES runs(id),
    tool_call_id TEXT UNIQUE NOT NULL REFERENCES tool_calls(id),
    status TEXT NOT NULL DEFAULT 'pending',
    expires TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS approvals_run_idx ON approvals(run_id);

-- Singleton user row (Desktop edition is single-user).
INSERT OR IGNORE INTO users (id, username, password_hash)
VALUES ('00000000-0000-0000-0000-000000000001', 'desktop', '');
