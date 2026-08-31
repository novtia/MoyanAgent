/** Latin text averages roughly four characters per token (matches Rust `estimate_text_tokens`). */
const ASCII_CHARS_PER_TOKEN = 4;

/**
 * Approximate token count of a text blob.
 *
 * CJK is counted at one token per character; ASCII uses `ceil(n / 4)`.
 * Same ratios as the agent loop's tokenizer-free estimator.
 */
export function estimateTextTokens(text: string): number {
  let ascii = 0;
  let wide = 0;
  for (const ch of text) {
    if (ch.charCodeAt(0) < 128) ascii += 1;
    else wide += 1;
  }
  return Math.floor((ascii + ASCII_CHARS_PER_TOKEN - 1) / ASCII_CHARS_PER_TOKEN) + wide;
}
