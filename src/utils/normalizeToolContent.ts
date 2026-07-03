/**
 * Mirror backend `normalize_tool_string`: strip JSON / HTML escape artifacts
 * from model tool `content` before diffing or displaying prose.
 */
export function normalizeToolContent(s: string): string {
  let cur = s;
  for (let pass = 0; pass < 8; pass += 1) {
    const next = normalizeToolContentOnce(cur);
    if (next === cur) break;
    cur = next;
  }
  while (cur.includes('\\"')) {
    cur = cur.replace(/\\"/g, '"');
  }
  cur = cur
    .replace(/&quot;/gi, '"')
    .replace(/&#34;/g, '"')
    .replace(/&apos;/gi, "'")
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
  return cur;
}

function normalizeToolContentOnce(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i += 1) {
    const ch = s[i];
    if (ch === "\\" && i + 1 < s.length) {
      const next = s[i + 1];
      switch (next) {
        case '"':
          out += '"';
          i += 1;
          continue;
        case "\\":
          out += "\\";
          i += 1;
          continue;
        case "n":
          out += "\n";
          i += 1;
          continue;
        case "t":
          out += "\t";
          i += 1;
          continue;
        case "r":
          out += "\r";
          i += 1;
          continue;
        default:
          out += ch;
          continue;
      }
    }
    out += ch;
  }
  return out;
}
