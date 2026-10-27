-- Local OpenAI-compatible endpoint (llama-server) and the rewrite model type.

INSERT OR IGNORE INTO llm_sdk_option (
  sdk_id, label, description, default_name, default_endpoint,
  endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint,
  model_id_placeholder, model_id_hint, sort_order
) VALUES (
  'local',
  '本地',
  '本机 OpenAI Chat Completions 兼容接口，例如 llama.cpp 的 llama-server。密钥可留空。阅读栏的改写、扩写和流水线使用类型为「改文」的模型。',
  '本地改文',
  'http://127.0.0.1:8080/v1/chat/completions',
  'http://127.0.0.1:8080/v1/chat/completions',
  '本地 llama-server 的完整 chat/completions 地址。',
  '可留空',
  'llama-server 默认不校验密钥，可留空。',
  'rewrite2-27b',
  '与 llama-server 的 -a 别名一致，例如 rewrite2-27b。',
  8
);

INSERT OR IGNORE INTO llm_supplier_preset (
  supplier_id, name, sdk_id, avatar, endpoint, enabled, sort_order
) VALUES (
  'local-rewrite',
  '本地改文',
  'local',
  '',
  'http://127.0.0.1:8080/v1/chat/completions',
  1,
  9
);

INSERT INTO llm_sdk_model (
  sdk_id, model_id, name, model_group, capabilities_json, sort_order, context_window
)
SELECT 'local', 'rewrite2-27b', '改文 27B', 'local', '["rewrite"]', 0, 4096
WHERE NOT EXISTS (
  SELECT 1 FROM llm_sdk_model WHERE sdk_id = 'local' AND model_id = 'rewrite2-27b'
);

INSERT INTO llm_supplier_model (
  supplier_id, model_id, name, model_group, capabilities_json, sort_order, context_window
)
SELECT 'local-rewrite', 'rewrite2-27b', '改文 27B', 'local', '["rewrite"]', 0, 4096
WHERE NOT EXISTS (
  SELECT 1 FROM llm_supplier_model
  WHERE supplier_id = 'local-rewrite' AND model_id = 'rewrite2-27b'
);

UPDATE llm_sdk_model
SET context_window = 4096
WHERE sdk_id = 'local' AND model_id = 'rewrite2-27b' AND context_window = 32768;

UPDATE llm_supplier_model
SET context_window = 4096
WHERE supplier_id = 'local-rewrite' AND model_id = 'rewrite2-27b' AND context_window = 32768;
