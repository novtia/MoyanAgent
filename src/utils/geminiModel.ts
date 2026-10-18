/**
 * Gemini model-id capability rules keyed off the parsed version number, so new
 * releases (3.8, 4, ...) work without editing call sites.
 *
 * Mirrors `src-tauri/src/ai/providers/gemini_model.rs`; keep both in sync.
 */

/** First version that ships thinking. */
const THINKING_MIN: readonly [number, number] = [2, 5];

/** Parse `[major, minor]` from ids like `gemini-3.8-flash` or `google/gemini-4-argon`. */
export function geminiVersion(modelId: string): [number, number] | null {
  const m = /gemini-(\d+)(?:\.(\d+))?/i.exec(modelId);
  if (!m) return null;
  return [Number(m[1]), m[2] ? Number(m[2]) : 0];
}

function isMediaModel(id: string): boolean {
  return id.includes("image") || id.includes("imagen") || id.includes("veo");
}

/** The model reasons by version alone (2.5 and every later release). */
export function geminiImpliesReasoning(modelId: string): boolean {
  const id = modelId.toLowerCase();
  if (isMediaModel(id)) return false;
  const v = geminiVersion(id);
  if (!v) return false;
  return v[0] > THINKING_MIN[0] || (v[0] === THINKING_MIN[0] && v[1] >= THINKING_MIN[1]);
}
