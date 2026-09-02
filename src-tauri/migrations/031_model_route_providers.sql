-- Persist OpenRouter `provider.only` slugs per model.
--
-- Catalog tables get a column for seed defaults. User edits (including models
-- added by hand that never appear in the catalog) live in `llm_model_route`.

ALTER TABLE llm_sdk_model ADD COLUMN route_providers_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE llm_supplier_model ADD COLUMN route_providers_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE IF NOT EXISTS llm_model_route (
  provider_id          TEXT NOT NULL,
  model_id             TEXT NOT NULL,
  route_providers_json TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (provider_id, model_id)
);
