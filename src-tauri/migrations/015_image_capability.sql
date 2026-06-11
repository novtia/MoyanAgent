-- atelier schema v15: tag built-in image-generation models with the new
-- "image" capability so the composer can gate aspect-ratio / image-size
-- controls on the selected model's type.
--
-- Appends "image" to capabilities_json for catalog rows whose model id looks
-- like an image model (and that don't already carry the tag).
UPDATE llm_sdk_model
SET capabilities_json = json_insert(capabilities_json, '$[#]', 'image')
WHERE capabilities_json NOT LIKE '%"image"%'
  AND (
    lower(model_id) LIKE '%image%'
    OR lower(model_id) LIKE '%seedream%'
    OR lower(model_id) LIKE '%imagine%'
    OR lower(model_id) LIKE '%dall%'
    OR lower(model_id) LIKE '%flux%'
  );

UPDATE llm_supplier_model
SET capabilities_json = json_insert(capabilities_json, '$[#]', 'image')
WHERE capabilities_json NOT LIKE '%"image"%'
  AND (
    lower(model_id) LIKE '%image%'
    OR lower(model_id) LIKE '%seedream%'
    OR lower(model_id) LIKE '%imagine%'
    OR lower(model_id) LIKE '%dall%'
    OR lower(model_id) LIKE '%flux%'
  );
