-- atelier schema baseline (squashed from historical v1–v27)
-- Fresh installs create the final schema in one shot.
PRAGMA foreign_keys = ON;

-- ─── settings ───────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- ─── projects (before sessions: sessions.project_id FK) ────────────────────
CREATE TABLE IF NOT EXISTS projects (
  id             TEXT PRIMARY KEY NOT NULL,
  name           TEXT NOT NULL,
  path           TEXT,
  sort_order     INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  system_prompt  TEXT NOT NULL DEFAULT '',
  history_turns  INTEGER NOT NULL DEFAULT 10,
  llm_params     TEXT,
  context_window INTEGER,
  agent_chain    TEXT
);

-- ─── sessions ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sessions (
  id                  TEXT PRIMARY KEY,
  title               TEXT NOT NULL,
  model               TEXT,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  system_prompt       TEXT NOT NULL DEFAULT '',
  history_turns       INTEGER NOT NULL DEFAULT 10,
  llm_params          TEXT,
  agent_type          TEXT NOT NULL DEFAULT 'general-purpose',
  context_window      INTEGER,
  context_window_used INTEGER NOT NULL DEFAULT 0,
  project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
  agent_chain         TEXT,
  provider_id         TEXT,
  parent_session_id   TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  is_temporary        INTEGER NOT NULL DEFAULT 0,
  spawn_task_id       TEXT,
  last_response_id    TEXT,
  cache_thinking_key  TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_sessions_temporary ON sessions(is_temporary);

-- ─── messages ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS messages (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role        TEXT NOT NULL,
  text        TEXT,
  params_json TEXT,
  created_at  INTEGER NOT NULL,
  events_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);

-- ─── message_images ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS message_images (
  id         TEXT PRIMARY KEY,
  message_id TEXT REFERENCES messages(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  role       TEXT NOT NULL,
  rel_path   TEXT NOT NULL,
  thumb_path TEXT,
  mime       TEXT NOT NULL,
  width      INTEGER,
  height     INTEGER,
  bytes      INTEGER,
  ord        INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL DEFAULT 0,
  media_role TEXT,
  source_url TEXT
);
CREATE INDEX IF NOT EXISTS idx_msgimg_msg ON message_images(message_id, ord);
CREATE INDEX IF NOT EXISTS idx_msgimg_session ON message_images(session_id);

-- ─── custom_agents ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS custom_agents (
  agent_type    TEXT PRIMARY KEY NOT NULL,
  name          TEXT NOT NULL,
  when_to_use   TEXT NOT NULL DEFAULT '',
  system_prompt TEXT NOT NULL DEFAULT '',
  model         TEXT,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  tools         TEXT
);

-- ─── role_state_snapshots ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS role_state_snapshots (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  state_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  scope_id   TEXT
);
CREATE INDEX IF NOT EXISTS idx_role_state_session ON role_state_snapshots(session_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_role_state_message ON role_state_snapshots(message_id);
CREATE INDEX IF NOT EXISTS idx_role_state_scope ON role_state_snapshots(scope_id, id);

-- ─── file_snapshots ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS file_snapshots (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id      TEXT NOT NULL,
  message_id      TEXT NOT NULL,
  path            TEXT NOT NULL,
  op              TEXT NOT NULL,
  before_existed  INTEGER NOT NULL,
  before_content  TEXT,
  restorable      INTEGER NOT NULL,
  created_at      INTEGER NOT NULL,
  before_encoding TEXT,
  before_had_bom  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_file_snapshots_session ON file_snapshots(session_id, id);
CREATE INDEX IF NOT EXISTS idx_file_snapshots_message ON file_snapshots(message_id);

-- ─── token_usage_events ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS token_usage_events (
  id                 TEXT PRIMARY KEY,
  created_at         INTEGER NOT NULL,
  event_kind         TEXT NOT NULL,
  session_id         TEXT,
  correlation_id     TEXT,
  message_id         TEXT,
  agent_id           TEXT,
  agent_type         TEXT,
  model              TEXT,
  provider           TEXT,
  turn_index         INTEGER,
  tool_name          TEXT,
  prompt_tokens      INTEGER,
  completion_tokens  INTEGER,
  total_tokens       INTEGER,
  output_chars       INTEGER,
  output_bytes       INTEGER,
  is_error           INTEGER NOT NULL DEFAULT 0,
  metadata_json      TEXT,
  content_json       TEXT,
  cache_read_tokens  INTEGER,
  cache_write_tokens INTEGER
);
CREATE INDEX IF NOT EXISTS idx_token_events_session ON token_usage_events(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_token_events_model ON token_usage_events(model, created_at);
CREATE INDEX IF NOT EXISTS idx_token_events_kind ON token_usage_events(event_kind, created_at);

-- ─── pending_diffs ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pending_diffs (
  id                 TEXT PRIMARY KEY,
  session_id         TEXT NOT NULL,
  path               TEXT NOT NULL,
  before_snippet     TEXT NOT NULL,
  after_snippet      TEXT NOT NULL,
  text_before        TEXT NOT NULL,
  text_after         TEXT NOT NULL,
  encoding           TEXT,
  had_bom            INTEGER NOT NULL DEFAULT 0,
  seq                INTEGER NOT NULL,
  created_at         INTEGER NOT NULL,
  request_message_id TEXT,
  message_id         TEXT
);
CREATE INDEX IF NOT EXISTS idx_pending_diffs_session_path ON pending_diffs(session_id, path, seq);
CREATE INDEX IF NOT EXISTS idx_pending_diffs_request_message ON pending_diffs(session_id, request_message_id);
CREATE INDEX IF NOT EXISTS idx_pending_diffs_message ON pending_diffs(session_id, message_id);

-- ─── LLM catalog ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS llm_sdk_option (
  sdk_id               TEXT NOT NULL PRIMARY KEY,
  label                TEXT NOT NULL,
  description          TEXT NOT NULL,
  default_name         TEXT NOT NULL,
  default_endpoint     TEXT NOT NULL,
  endpoint_placeholder TEXT NOT NULL,
  endpoint_hint        TEXT NOT NULL,
  api_key_placeholder  TEXT NOT NULL,
  api_key_hint         TEXT NOT NULL,
  model_id_placeholder TEXT NOT NULL,
  model_id_hint        TEXT NOT NULL,
  sort_order           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS llm_sdk_model (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  sdk_id            TEXT NOT NULL REFERENCES llm_sdk_option(sdk_id) ON DELETE CASCADE,
  model_id          TEXT NOT NULL,
  name              TEXT NOT NULL,
  model_group       TEXT NOT NULL,
  capabilities_json TEXT NOT NULL,
  sort_order        INTEGER NOT NULL DEFAULT 0,
  context_window    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_llm_sdk_model_sdk ON llm_sdk_model(sdk_id, sort_order);

CREATE TABLE IF NOT EXISTS llm_supplier_preset (
  supplier_id TEXT NOT NULL PRIMARY KEY,
  name        TEXT NOT NULL,
  sdk_id      TEXT NOT NULL REFERENCES llm_sdk_option(sdk_id),
  avatar      TEXT NOT NULL,
  endpoint    TEXT NOT NULL,
  enabled     INTEGER NOT NULL DEFAULT 0,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS llm_supplier_model (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  supplier_id       TEXT NOT NULL REFERENCES llm_supplier_preset(supplier_id) ON DELETE CASCADE,
  model_id          TEXT NOT NULL,
  name              TEXT NOT NULL,
  model_group       TEXT NOT NULL,
  capabilities_json TEXT NOT NULL,
  sort_order        INTEGER NOT NULL DEFAULT 0,
  context_window    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_llm_supplier_model_sup ON llm_supplier_model(supplier_id, sort_order);

-- Seed: SDK options (006 + 021)
INSERT OR IGNORE INTO llm_sdk_option (
  sdk_id, label, description, default_name, default_endpoint,
  endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint,
  model_id_placeholder, model_id_hint, sort_order
) VALUES
('openai', 'OpenAI Chat', 'OpenAI Chat Completions 兼容协议，OpenRouter 也走这个 SDK。', 'OpenAI Chat', 'https://api.openai.com/v1/chat/completions', 'https://.../chat/completions', '填写完整 chat/completions 地址；OpenRouter 使用 https://openrouter.ai/api/v1/chat/completions。', 'sk-...', '填写该供应商的 API Key。', 'model-name', '填写该供应商的模型 ID；OpenRouter 使用 provider/model-name。', 0),
('openai-responses', 'OpenAI Responses', 'OpenAI Responses API，支持文本和图片输入，适合 OpenAI 原生新接口。', 'OpenAI', 'https://api.openai.com/v1/responses', 'https://api.openai.com/v1/responses', 'OpenAI Responses API 的完整地址。', 'sk-...', '填写 OpenAI API Key。', 'gpt-4.1', '填写 OpenAI Responses 支持的模型 ID。', 1),
('gemini', 'Gemini', 'Google Gemini generateContent API，支持文本、图片输入和 Gemini 图片输出。', 'Gemini', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', '保留 {model} 占位符；后端会替换为当前模型 ID。', 'AIza...', '填写 Gemini API Key。', 'gemini-2.5-flash-image', '填写 Gemini 模型 ID。', 2),
('claude', 'Claude', 'Anthropic Messages API，支持文本和图片输入。', 'Claude', 'https://api.anthropic.com/v1/messages', 'https://api.anthropic.com/v1/messages', 'Anthropic Messages API 的完整地址。', 'sk-ant-...', '填写 Anthropic API Key。', 'claude-sonnet-4-20250514', '填写 Anthropic 模型 ID。', 3),
('grok', 'xAI Grok Image', 'xAI 原生图片 API（/v1/images/generations 与 /v1/images/edits），非 OpenAI Chat 兼容层。', 'xAI Grok', 'https://api.x.ai/v1/images/generations', 'https://api.x.ai/v1/images/generations', '使用 xAI 图片生成完整地址；编辑请求会自动改用同前缀下的 …/images/edits。也可填 https://api.x.ai/v1 作为前缀。', 'xai-...', '填写 xAI（Grok）API Key。', 'grok-imagine-image-quality', '填写 Grok Imagine 图片模型 ID（见 xAI 文档）。', 4),
('ark-images', '豆包生图', '豆包 Seedream 等模型的图片生成接口（POST …/api/v3/images/generations）。不能与 chat/completions 混用；若误填对话地址，后端会自动改为生图地址。', '豆包生图', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', '在豆包/方舟控制台使用「图片生成」对应的 Endpoint；若只填到 …/api/v3 也会自动补上 /images/generations。误填 …/chat/completions 时也会自动替换为生图路径。', 'API Key', '与豆包（火山引擎方舟）控制台中的 API Key 一致（Bearer）。', 'doubao-seedream-5-0-260128', '填写控制台中该生图模型的接入点 ID（如 doubao-seedream-*）。', 5),
('ark-video', '豆包生视频', '火山方舟 / BytePlus Seedance 异步视频生成接口，支持文生视频、首尾帧和多模态参考。', '豆包生视频', 'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks', 'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks', '国内火山方舟使用 cn-beijing 地址；BytePlus 可改为 https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks。', 'API Key', '填写火山方舟或 BytePlus ModelArk API Key（Bearer）。', 'doubao-seedance-2-0-260128', '填写 Seedance 模型 ID 或推理接入点 ID。', 6);

-- Seed: SDK models (006 + 015 image tags + 021 video)
INSERT INTO llm_sdk_model (sdk_id, model_id, name, model_group, capabilities_json, sort_order, context_window) VALUES
('openai', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 0, NULL),
('openai', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1, NULL),
('openai-responses', 'gpt-image-1.5', 'GPT Image 1.5', 'openai', '["vision","image"]', 0, NULL),
('openai-responses', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1, NULL),
('openai-responses', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 2, NULL),
('gemini', 'gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'gemini', '["vision","text","image"]', 0, NULL),
('gemini', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text"]', 1, NULL),
('gemini', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2, NULL),
('claude', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', 'claude', '["vision","text","reasoning"]', 0, NULL),
('claude', 'claude-opus-4-1-20250805', 'Claude Opus 4.1', 'claude', '["vision","text","reasoning"]', 1, NULL),
('grok', 'grok-imagine-image-quality', 'Grok Imagine (quality)', 'grok', '["vision","text","image"]', 0, NULL),
('ark-images', 'doubao-seedream-5-0-260128', '豆包 Seedream 5.0', 'doubao', '["vision","text","image"]', 0, NULL),
('ark-video', 'doubao-seedance-2-0-260128', '豆包 Seedance 2.0', 'doubao', '["video","multimodal-ref"]', 0, NULL),
('ark-video', 'doubao-seedance-1-5-pro-251215', '豆包 Seedance 1.5 Pro', 'doubao', '["video"]', 1, NULL),
('ark-video', 'seedance-2-0-260128', 'Seedance 2.0 (BytePlus)', 'byteplus', '["video","multimodal-ref"]', 2, NULL),
('ark-video', 'seedance-1-5-pro-251215', 'Seedance 1.5 Pro (BytePlus)', 'byteplus', '["video"]', 3, NULL);

-- Seed: supplier presets
INSERT OR IGNORE INTO llm_supplier_preset (supplier_id, name, sdk_id, avatar, endpoint, enabled, sort_order) VALUES
('openrouter', 'OpenRouter', 'openai', '/provider-icons/openrouter.svg', 'https://openrouter.ai/api/v1/chat/completions', 1, 0),
('openai', 'OpenAI', 'openai-responses', '/provider-icons/openai.svg', 'https://api.openai.com/v1/responses', 0, 1),
('gemini', 'Gemini', 'gemini', '/provider-icons/gemini.svg', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', 0, 2),
('claude', 'Claude', 'claude', '/provider-icons/claude.svg', 'https://api.anthropic.com/v1/messages', 0, 3),
('grok', 'xAI Grok', 'grok', '/provider-icons/grok.svg', 'https://api.x.ai/v1/images/generations', 0, 4),
('volcengine-ark', '豆包生图', 'ark-images', '/provider-icons/doubao-color.svg', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', 0, 5),
('deepseek', 'DeepSeek', 'openai', '/provider-icons/deepseek.svg', 'https://api.deepseek.com/chat/completions', 0, 6),
('volcengine-ark-video', '豆包生视频', 'ark-video', '/provider-icons/doubao-color.svg', 'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks', 0, 7);

-- Seed: supplier models (015 image + 008 deepseek context_window + 021 video)
INSERT INTO llm_supplier_model (supplier_id, model_id, name, model_group, capabilities_json, sort_order, context_window) VALUES
('openrouter', 'openai/gpt-5.4-image-2', 'GPT Image 2', 'openai', '["vision","text","image"]', 0, NULL),
('openrouter', 'google/gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'google', '["vision","text","image"]', 1, NULL),
('openai', 'gpt-image-1.5', 'GPT Image 1.5', 'openai', '["vision","image"]', 0, NULL),
('openai', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1, NULL),
('openai', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 2, NULL),
('gemini', 'gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'gemini', '["vision","text","image"]', 0, NULL),
('gemini', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text"]', 1, NULL),
('gemini', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2, NULL),
('claude', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', 'claude', '["vision","text","reasoning"]', 0, NULL),
('claude', 'claude-opus-4-1-20250805', 'Claude Opus 4.1', 'claude', '["vision","text","reasoning"]', 1, NULL),
('grok', 'grok-imagine-image-quality', 'Grok Imagine (quality)', 'grok', '["vision","text","image"]', 0, NULL),
('volcengine-ark', 'doubao-seedream-5-0-260128', '豆包 Seedream 5.0', 'doubao', '["vision","text","image"]', 0, NULL),
('deepseek', 'deepseek-v4-flash', 'DeepSeek V4 Flash', 'deepseek', '["text","reasoning"]', 0, 1000000),
('deepseek', 'deepseek-v4-pro', 'DeepSeek V4 Pro', 'deepseek', '["text","reasoning"]', 1, 1000000),
('volcengine-ark-video', 'doubao-seedance-2-0-260128', '豆包 Seedance 2.0', 'doubao', '["video","multimodal-ref"]', 0, NULL),
('volcengine-ark-video', 'doubao-seedance-1-5-pro-251215', '豆包 Seedance 1.5 Pro', 'doubao', '["video"]', 1, NULL);