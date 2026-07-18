//! Portable SQL DDL for the two tiers (§6.2) + the schema version. The schema is
//! a STABLE, DENORMALIZED read contract (§6.1) — Bridge reads these tables.

/// Bumped only on a breaking schema change; gates per-file migration (§6.3).
pub const SCHEMA_VERSION: u32 = 3; // boards + board_items (B-1)

/// `lens.db` — control plane (one file). `meta` is created by `db::open_db`.
/// P2 additions vs §6.2 sketch: `cost_json` (D-P2-2), `terminal_pending` (D-P2-3).
pub const CONTROL_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS connections (
  id          TEXT PRIMARY KEY,
  base_url    TEXT NOT NULL,
  auth_kind   TEXT NOT NULL,
  label       TEXT,
  server_info TEXT,
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  connection_id     TEXT NOT NULL REFERENCES connections(id),
  id                TEXT NOT NULL,
  agent_id          TEXT NOT NULL,
  agent_name        TEXT,
  runner_id         TEXT,
  parent_session_id TEXT,
  status            TEXT NOT NULL,
  last_task_error   TEXT,
  llm_model         TEXT,
  model_override    TEXT,
  reasoning_effort  TEXT,
  collaboration_mode TEXT,
  context_window    INTEGER,
  last_total_tokens INTEGER,
  cumulative_cost   REAL,
  usage_by_model    TEXT,
  cost_json         TEXT,
  workspace         TEXT,
  git_branch        TEXT,
  host_type         TEXT NOT NULL,
  host_id           TEXT,
  title             TEXT,
  labels            TEXT,
  permission_level  INTEGER,
  owner             TEXT,
  todos             TEXT,
  skills            TEXT,
  terminal_pending  INTEGER NOT NULL DEFAULT 0,
  created_at        INTEGER NOT NULL,
  archived          INTEGER NOT NULL DEFAULT 0,
  lifecycle         TEXT NOT NULL DEFAULT 'active',
  pinned            INTEGER NOT NULL DEFAULT 0,
  tombstoned_at     INTEGER,
  last_focused_at   INTEGER,
  last_status       TEXT,
  updated_at        INTEGER NOT NULL,
  PRIMARY KEY (connection_id, id)
);

CREATE TABLE IF NOT EXISTS cost_samples (
  connection_id  TEXT NOT NULL,
  session_id     TEXT NOT NULL,
  sampled_at     INTEGER NOT NULL,
  total_cost_usd REAL NOT NULL,
  PRIMARY KEY (connection_id, session_id, sampled_at)
);

CREATE TABLE IF NOT EXISTS boards (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  ordinal    INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS board_items (
  item_id        TEXT PRIMARY KEY,
  board_id       TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  parent_item_id TEXT REFERENCES board_items(item_id) ON DELETE CASCADE,
  ordinal        INTEGER NOT NULL,
  kind           TEXT NOT NULL,

  session_conn_id TEXT,
  session_id      TEXT,

  group_name   TEXT,
  color_token  TEXT,
  collapsed    INTEGER NOT NULL DEFAULT 0,
  archived     INTEGER NOT NULL DEFAULT 0,
  group_config TEXT,

  created_at   INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS board_items_session
  ON board_items(session_conn_id, session_id) WHERE kind = 'card';

CREATE INDEX IF NOT EXISTS board_items_parent ON board_items(board_id, parent_item_id, ordinal);
"#;

/// Per-session transcript file. `meta` (created by `db::open_db`) additionally
/// carries `connection_id` + `session_id` so the file is self-describing (§6.2).
pub const TRANSCRIPT_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS items (
  item_id     TEXT NOT NULL,
  live_seq    INTEGER,
  ordinal     INTEGER NOT NULL,
  kind        TEXT NOT NULL,
  payload     TEXT NOT NULL,
  agent       TEXT,
  depth       INTEGER NOT NULL DEFAULT 0,
  turn        INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER NOT NULL,
  provisional INTEGER NOT NULL DEFAULT 0,
  call_id     TEXT,
  PRIMARY KEY (item_id),
  UNIQUE (ordinal)
);
"#;
