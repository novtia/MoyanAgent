-- atelier schema v21: Seedance video provider and generic media metadata.
PRAGMA foreign_keys = ON;

ALTER TABLE message_images ADD COLUMN media_role TEXT;
ALTER TABLE message_images ADD COLUMN source_url TEXT;

INSERT OR IGNORE INTO llm_sdk_option (
  sdk_id, label, description, default_name, default_endpoint,
  endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint,
  model_id_placeholder, model_id_hint, sort_order
) VALUES (
  'ark-video',
  '豆包生视频',
  '火山方舟 / BytePlus Seedance 异步视频生成接口，支持文生视频、首尾帧和多模态参考。',
  '豆包生视频',
  'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks',
  'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks',
  '国内火山方舟使用 cn-beijing 地址；BytePlus 可改为 https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks。',
  'API Key',
  '填写火山方舟或 BytePlus ModelArk API Key（Bearer）。',
  'doubao-seedance-2-0-260128',
  '填写 Seedance 模型 ID 或推理接入点 ID。',
  6
);

INSERT INTO llm_sdk_model (
  sdk_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'ark-video', 'doubao-seedance-2-0-260128', '豆包 Seedance 2.0', 'doubao',
       '["video","multimodal-ref"]', 0
WHERE NOT EXISTS (
  SELECT 1 FROM llm_sdk_model
  WHERE sdk_id='ark-video' AND model_id='doubao-seedance-2-0-260128'
);

INSERT INTO llm_sdk_model (
  sdk_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'ark-video', 'doubao-seedance-1-5-pro-251215', '豆包 Seedance 1.5 Pro', 'doubao',
       '["video"]', 1
WHERE NOT EXISTS (
  SELECT 1 FROM llm_sdk_model
  WHERE sdk_id='ark-video' AND model_id='doubao-seedance-1-5-pro-251215'
);

INSERT INTO llm_sdk_model (
  sdk_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'ark-video', 'seedance-2-0-260128', 'Seedance 2.0 (BytePlus)', 'byteplus',
       '["video","multimodal-ref"]', 2
WHERE NOT EXISTS (
  SELECT 1 FROM llm_sdk_model
  WHERE sdk_id='ark-video' AND model_id='seedance-2-0-260128'
);

INSERT INTO llm_sdk_model (
  sdk_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'ark-video', 'seedance-1-5-pro-251215', 'Seedance 1.5 Pro (BytePlus)', 'byteplus',
       '["video"]', 3
WHERE NOT EXISTS (
  SELECT 1 FROM llm_sdk_model
  WHERE sdk_id='ark-video' AND model_id='seedance-1-5-pro-251215'
);

INSERT OR IGNORE INTO llm_supplier_preset (
  supplier_id, name, sdk_id, avatar, endpoint, enabled, sort_order
) VALUES (
  'volcengine-ark-video',
  '豆包生视频',
  'ark-video',
  '/provider-icons/doubao-color.svg',
  'https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks',
  0,
  7
);

INSERT INTO llm_supplier_model (
  supplier_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'volcengine-ark-video', 'doubao-seedance-2-0-260128', '豆包 Seedance 2.0',
       'doubao', '["video","multimodal-ref"]', 0
WHERE NOT EXISTS (
  SELECT 1 FROM llm_supplier_model
  WHERE supplier_id='volcengine-ark-video'
    AND model_id='doubao-seedance-2-0-260128'
);

INSERT INTO llm_supplier_model (
  supplier_id, model_id, name, model_group, capabilities_json, sort_order
)
SELECT 'volcengine-ark-video', 'doubao-seedance-1-5-pro-251215',
       '豆包 Seedance 1.5 Pro', 'doubao', '["video"]', 1
WHERE NOT EXISTS (
  SELECT 1 FROM llm_supplier_model
  WHERE supplier_id='volcengine-ark-video'
    AND model_id='doubao-seedance-1-5-pro-251215'
);

UPDATE llm_sdk_model
SET capabilities_json = json_insert(capabilities_json, '$[#]', 'video')
WHERE lower(model_id) LIKE '%seedance%'
  AND capabilities_json NOT LIKE '%"video"%';

UPDATE llm_supplier_model
SET capabilities_json = json_insert(capabilities_json, '$[#]', 'video')
WHERE lower(model_id) LIKE '%seedance%'
  AND capabilities_json NOT LIKE '%"video"%';
