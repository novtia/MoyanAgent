-- Vertex AI Gemini generateContent catalog (SDK + builtin supplier).
-- Idempotent: llm_sdk_option / llm_supplier_preset use INSERT OR IGNORE.
-- Model rows are only inserted when the vertex SDK option is missing.

INSERT OR IGNORE INTO llm_sdk_option (
  sdk_id, label, description, default_name, default_endpoint,
  endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint,
  model_id_placeholder, model_id_hint, sort_order
) VALUES (
  'vertex',
  'Vertex AI',
  'Google Cloud Vertex AI 的 Gemini generateContent 接口。填写 GCP 项目 ID 和区域后会自动拼 URL；密钥支持 API Key 或 gcloud access token。',
  'Vertex AI',
  'https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent',
  'https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent',
  '用设置里的项目 ID 和区域拼好地址，并保留 {model}；后端会替换为当前模型 ID，流式时改为 streamGenerateContent。',
  'AIza... 或 ya29...',
  '填写 Google Cloud API Key，或 gcloud auth print-access-token 得到的 access token。',
  'gemini-2.5-flash',
  '填写 Vertex 上的 Gemini 模型 ID（不含 google/ 前缀）。',
  7
);

INSERT OR IGNORE INTO llm_supplier_preset (supplier_id, name, sdk_id, avatar, endpoint, enabled, sort_order) VALUES
('vertex', 'Vertex AI', 'vertex', '', 'https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent', 0, 8);

INSERT INTO llm_sdk_model (sdk_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text","reasoning"]', 0, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_sdk_model WHERE sdk_id = 'vertex' AND model_id = 'gemini-2.5-flash');

INSERT INTO llm_sdk_model (sdk_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-2.5-pro', 'Gemini 2.5 Pro', 'gemini', '["vision","text","reasoning"]', 1, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_sdk_model WHERE sdk_id = 'vertex' AND model_id = 'gemini-2.5-pro');

INSERT INTO llm_sdk_model (sdk_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_sdk_model WHERE sdk_id = 'vertex' AND model_id = 'gemini-3-flash-preview');

INSERT INTO llm_supplier_model (supplier_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-2.5-flash', 'Gemini 2.5 Flash', 'gemini', '["vision","text","reasoning"]', 0, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_supplier_model WHERE supplier_id = 'vertex' AND model_id = 'gemini-2.5-flash');

INSERT INTO llm_supplier_model (supplier_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-2.5-pro', 'Gemini 2.5 Pro', 'gemini', '["vision","text","reasoning"]', 1, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_supplier_model WHERE supplier_id = 'vertex' AND model_id = 'gemini-2.5-pro');

INSERT INTO llm_supplier_model (supplier_id, model_id, name, model_group, capabilities_json, sort_order, context_window)
SELECT 'vertex', 'gemini-3-flash-preview', 'Gemini 3 Flash Preview', 'gemini', '["vision","text","reasoning"]', 2, NULL
WHERE NOT EXISTS (SELECT 1 FROM llm_supplier_model WHERE supplier_id = 'vertex' AND model_id = 'gemini-3-flash-preview');
