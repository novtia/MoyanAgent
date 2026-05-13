-- DeepSeek preset models: ~1M token context window for UI / session limits
PRAGMA foreign_keys = ON;

UPDATE llm_supplier_model
SET context_window = 1000000
WHERE supplier_id = 'deepseek';
