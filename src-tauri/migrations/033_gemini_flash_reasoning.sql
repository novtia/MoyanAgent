-- Gemini 2.5 Flash supports thinking; catalog rows previously omitted reasoning
-- so the composer hid the thinking toggle. Thought summaries still require
-- generationConfig.thinkingConfig.includeThoughts on Vertex / Gemini.

UPDATE llm_sdk_model
SET capabilities_json = '["vision","text","reasoning"]'
WHERE model_id = 'gemini-2.5-flash'
  AND sdk_id IN ('gemini', 'vertex');

UPDATE llm_supplier_model
SET capabilities_json = '["vision","text","reasoning"]'
WHERE model_id = 'gemini-2.5-flash'
  AND supplier_id IN ('gemini', 'vertex');
