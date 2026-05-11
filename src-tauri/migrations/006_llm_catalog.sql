-- atelier schema v6: LLM SDK catalog + builtin supplier presets (seed for UI / merge)
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS llm_sdk_option (
  sdk_id TEXT NOT NULL PRIMARY KEY,
  label TEXT NOT NULL,
  description TEXT NOT NULL,
  default_name TEXT NOT NULL,
  default_endpoint TEXT NOT NULL,
  endpoint_placeholder TEXT NOT NULL,
  endpoint_hint TEXT NOT NULL,
  api_key_placeholder TEXT NOT NULL,
  api_key_hint TEXT NOT NULL,
  model_id_placeholder TEXT NOT NULL,
  model_id_hint TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS llm_sdk_model (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  sdk_id TEXT NOT NULL REFERENCES llm_sdk_option(sdk_id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  name TEXT NOT NULL,
  model_group TEXT NOT NULL,
  capabilities_json TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_llm_sdk_model_sdk ON llm_sdk_model(sdk_id, sort_order);

CREATE TABLE IF NOT EXISTS llm_supplier_preset (
  supplier_id TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  sdk_id TEXT NOT NULL REFERENCES llm_sdk_option(sdk_id),
  avatar TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS llm_supplier_model (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  supplier_id TEXT NOT NULL REFERENCES llm_supplier_preset(supplier_id) ON DELETE CASCADE,
  model_id TEXT NOT NULL,
  name TEXT NOT NULL,
  model_group TEXT NOT NULL,
  capabilities_json TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_llm_supplier_model_sup ON llm_supplier_model(supplier_id, sort_order);

INSERT INTO llm_sdk_option (sdk_id, label, description, default_name, default_endpoint, endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint, model_id_placeholder, model_id_hint, sort_order) VALUES
('openai', 'OpenAI Chat', 'OpenAI Chat Completions 兼容协议，OpenRouter 也走这个 SDK。', 'OpenAI Chat', 'https://api.openai.com/v1/chat/completions', 'https://.../chat/completions', '填写完整 chat/completions 地址；OpenRouter 使用 https://openrouter.ai/api/v1/chat/completions。', 'sk-...', '填写该供应商的 API Key。', 'model-name', '填写该供应商的模型 ID；OpenRouter 使用 provider/model-name。', 0),
('openai-responses', 'OpenAI Responses', 'OpenAI Responses API，支持文本和图片输入，适合 OpenAI 原生新接口。', 'OpenAI', 'https://api.openai.com/v1/responses', 'https://api.openai.com/v1/responses', 'OpenAI Responses API 的完整地址。', 'sk-...', '填写 OpenAI API Key。', 'gpt-4.1', '填写 OpenAI Responses 支持的模型 ID。', 1),
('gemini', 'Gemini', 'Google Gemini generateContent API，支持文本、图片输入和 Gemini 图片输出。', 'Gemini', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', '保留 {model} 占位符；后端会替换为当前模型 ID。', 'AIza...', '填写 Gemini API Key。', 'gemini-2.5-flash-image', '填写 Gemini 模型 ID。', 2),
('claude', 'Claude', 'Anthropic Messages API，支持文本和图片输入。', 'Claude', 'https://api.anthropic.com/v1/messages', 'https://api.anthropic.com/v1/messages', 'Anthropic Messages API 的完整地址。', 'sk-ant-...', '填写 Anthropic API Key。', 'claude-sonnet-4-20250514', '填写 Anthropic 模型 ID。', 3),
('grok', 'xAI Grok Image', 'xAI 原生图片 API（/v1/images/generations 与 /v1/images/edits），非 OpenAI Chat 兼容层。', 'xAI Grok', 'https://api.x.ai/v1/images/generations', 'https://api.x.ai/v1/images/generations', '使用 xAI 图片生成完整地址；编辑请求会自动改用同前缀下的 …/images/edits。也可填 https://api.x.ai/v1 作为前缀。', 'xai-...', '填写 xAI（Grok）API Key。', 'grok-imagine-image-quality', '填写 Grok Imagine 图片模型 ID（见 xAI 文档）。', 4),
('ark-images', '豆包生图', '豆包 Seedream 等模型的图片生成接口（POST …/api/v3/images/generations）。不能与 chat/completions 混用；若误填对话地址，后端会自动改为生图地址。', '豆包生图', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', '在豆包/方舟控制台使用「图片生成」对应的 Endpoint；若只填到 …/api/v3 也会自动补上 /images/generations。误填 …/chat/completions 时也会自动替换为生图路径。', 'API Key', '与豆包（火山引擎方舟）控制台中的 API Key 一致（Bearer）。', 'doubao-seedream-5-0-260128', '填写控制台中该生图模型的接入点 ID（如 doubao-seedream-*）。', 5);

INSERT INTO llm_sdk_model (sdk_id, model_id, name, model_group, capabilities_json, sort_order) VALUES
('openai', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 0),
('openai', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1),
('openai-responses', 'gpt-image-1.5', 'GPT Image 1.5', 'openai', '["vision"]', 0),
('openai-responses', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1),
('openai-responses', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 2),
('gemini', 'gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'gemini', '["vision","text"]', 0),
('gemini', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text"]', 1),
('gemini', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2),
('claude', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', 'claude', '["vision","text","reasoning"]', 0),
('claude', 'claude-opus-4-1-20250805', 'Claude Opus 4.1', 'claude', '["vision","text","reasoning"]', 1),
('grok', 'grok-imagine-image-quality', 'Grok Imagine (quality)', 'grok', '["vision","text"]', 0),
('ark-images', 'doubao-seedream-5-0-260128', '豆包 Seedream 5.0', 'doubao', '["vision","text"]', 0);

INSERT INTO llm_supplier_preset (supplier_id, name, sdk_id, avatar, endpoint, enabled, sort_order) VALUES
('openrouter', 'OpenRouter', 'openai', '/provider-icons/openrouter.svg', 'https://openrouter.ai/api/v1/chat/completions', 1, 0),
('openai', 'OpenAI', 'openai-responses', '/provider-icons/openai.svg', 'https://api.openai.com/v1/responses', 0, 1),
('gemini', 'Gemini', 'gemini', '/provider-icons/gemini.svg', 'https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent', 0, 2),
('claude', 'Claude', 'claude', '/provider-icons/claude.svg', 'https://api.anthropic.com/v1/messages', 0, 3),
('grok', 'xAI Grok', 'grok', '/provider-icons/grok.svg', 'https://api.x.ai/v1/images/generations', 0, 4),
('volcengine-ark', '豆包生图', 'ark-images', '/provider-icons/doubao-color.svg', 'https://ark.cn-beijing.volces.com/api/v3/images/generations', 0, 5),
('deepseek', 'DeepSeek', 'openai', '/provider-icons/deepseek.svg', 'https://api.deepseek.com/chat/completions', 0, 6);

INSERT INTO llm_supplier_model (supplier_id, model_id, name, model_group, capabilities_json, sort_order) VALUES
('openrouter', 'openai/gpt-5.4-image-2', 'GPT Image 2', 'openai', '["vision","text"]', 0),
('openrouter', 'google/gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'google', '["vision","text"]', 1),
('openai', 'gpt-image-1.5', 'GPT Image 1.5', 'openai', '["vision"]', 0),
('openai', 'gpt-4.1', 'GPT 4.1', 'openai', '["vision","text"]', 1),
('openai', 'gpt-4o', 'GPT 4o', 'openai', '["vision","text"]', 2),
('gemini', 'gemini-2.5-flash-image', 'Gemini 2.5 Flash Image', 'gemini', '["vision","text"]', 0),
('gemini', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text"]', 1),
('gemini', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2),
('claude', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', 'claude', '["vision","text","reasoning"]', 0),
('claude', 'claude-opus-4-1-20250805', 'Claude Opus 4.1', 'claude', '["vision","text","reasoning"]', 1),
('grok', 'grok-imagine-image-quality', 'Grok Imagine (quality)', 'grok', '["vision","text"]', 0),
('volcengine-ark', 'doubao-seedream-5-0-260128', '豆包 Seedream 5.0', 'doubao', '["vision","text"]', 0),
('deepseek', 'deepseek-v4-flash', 'DeepSeek V4 Flash', 'deepseek', '["text","reasoning"]', 0),
('deepseek', 'deepseek-v4-pro', 'DeepSeek V4 Pro', 'deepseek', '["text","reasoning"]', 1);
