/**
 * Single source of truth for `@file` mention "reference cards".
 *
 * All mention logic — path normalization, icon selection, serialization to/from
 * the `@\"…\"` text form, the contenteditable chip DOM node, and project-scope
 * validation — lives here so the editor, the static renderer and the drop
 * handlers all behave identically. No React in this file (pure + DOM helpers).
 */

export const MENTION_PREFIX = "@";

/**
 * Matches a serialized mention in plain text (`@"…"` form only).
 * Prefer {@link parseMentionSegments} for rendering and {@link parseMentionPaths}
 * for path extraction.
 */
export const MENTION_RE = /@"((?:[^"\\]|\\.)*)"/g;

export type MentionSegment =
  | { type: "text"; value: string }
  | { type: "mention"; path: string };

export type MentionIconKind = "folder" | "image" | "code" | "file";

const IMAGE_EXT = /^(png|jpe?g|gif|webp|bmp|svg|ico|tiff?|avif)$/;
const CODE_EXT =
  /^(ts|tsx|js|jsx|mjs|cjs|rs|py|go|java|kt|c|cc|cpp|h|hpp|cs|rb|php|swift|css|scss|less|html|json|toml|yaml|yml|xml|sh|sql)$/;

/** Inner SVG markup per icon kind — shared by DOM chips and the React icon. */
export const MENTION_ICON_INNER: Record<MentionIconKind, string> = {
  folder:
    '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  image:
    '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-4.35-4.35a2 2 0 0 0-2.83 0L3 21"/>',
  code: '<polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>',
  file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
};

/** Strip the Windows extended-length prefix (`\\?\`, `\\?\UNC\`) for clean paths. */
export function normalizeMentionPath(p: string): string {
  const t = p.trim();
  if (/^\\\\\?\\UNC\\/i.test(t)) return `\\\\${t.slice(8)}`;
  if (/^\\\\\?\\/.test(t)) return t.slice(4);
  return t;
}

/** Last path segment (file or folder name) of an absolute path. */
export function mentionBasename(p: string): string {
  const s = p.replace(/[/\\]+$/, "");
  const i = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
  return (i >= 0 ? s.slice(i + 1) : s) || s;
}

/** Heuristic: a path whose last segment has no extension is treated as a folder. */
export function looksLikeDir(absPath: string): boolean {
  return !mentionBasename(absPath).includes(".");
}

/** Resolve the icon kind from the path kind / file extension. */
export function mentionIconKind(absPath: string, isDir?: boolean): MentionIconKind {
  if (isDir ?? looksLikeDir(absPath)) return "folder";
  const name = mentionBasename(absPath).toLowerCase();
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  if (IMAGE_EXT.test(ext)) return "image";
  if (CODE_EXT.test(ext)) return "code";
  return "file";
}

/** Full `<svg>` markup string for a path (used by the contenteditable DOM chip). */
export function mentionIconSvg(absPath: string, isDir?: boolean): string {
  const inner = MENTION_ICON_INNER[mentionIconKind(absPath, isDir)];
  return `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
}

/** Serialize one mention path into the `@\"…\"` plain-text form stored in messages. */
export function serializeMentionPath(path: string): string {
  return `${MENTION_PREFIX}${JSON.stringify(path)}`;
}

const QUOTED_MENTION_HEAD = /^@"((?:[^"\\]|\\.)*)"/;

/** Decode the inner payload of a quoted `@\"…\"` mention. */
function decodeQuotedMentionPayload(raw: string): string {
  return JSON.parse(`"${raw}"`) as string;
}

/** Parse a single `@\"…\"` mention starting at `atIndex` (which must point to `@`). */
export function parseMentionAt(
  text: string,
  atIndex: number,
): { path: string; length: number } | null {
  if (text[atIndex] !== MENTION_PREFIX) return null;
  const rest = text.slice(atIndex);

  const quoted = rest.match(QUOTED_MENTION_HEAD);
  if (quoted) {
    return {
      path: normalizeMentionPath(decodeQuotedMentionPayload(quoted[1])),
      length: quoted[0].length,
    };
  }

  return null;
}

/** Split plain text into alternating text / mention segments. */
export function parseMentionSegments(text: string): MentionSegment[] {
  const segments: MentionSegment[] = [];
  if (!text) return segments;

  let i = 0;
  while (i < text.length) {
    const at = text.indexOf(MENTION_PREFIX, i);
    if (at === -1) {
      segments.push({ type: "text", value: text.slice(i) });
      break;
    }
    if (at > i) {
      segments.push({ type: "text", value: text.slice(i, at) });
    }
    const parsed = parseMentionAt(text, at);
    if (parsed) {
      segments.push({ type: "mention", path: parsed.path });
      i = at + parsed.length;
      continue;
    }
    segments.push({ type: "text", value: MENTION_PREFIX });
    i = at + 1;
  }
  return segments;
}

/** Extract the mention paths from serialized text, in document order. */
export function parseMentionPaths(text: string): string[] {
  return parseMentionSegments(text)
    .filter((s): s is Extract<MentionSegment, { type: "mention" }> => s.type === "mention")
    .map((s) => s.path);
}

/* ───── project-scope validation ───── */

function normalizePath(p: string): string {
  return p.replace(/[/\\]+$/, "").replace(/\\/g, "/").toLowerCase();
}

/** True when `path` is the project root or sits inside it. */
export function isWithinProject(path: string, root: string): boolean {
  const np = normalizePath(path);
  const nr = normalizePath(root);
  return np === nr || np.startsWith(`${nr}/`);
}

/* ───── contenteditable chip DOM + serialization ───── */

/**
 * Build the inline, non-editable mention chip for a file path.
 * Layout: file-type icon, file name, remove button. The full path lives on
 * `dataset.path` (used for serialization), while only the name is displayed.
 */
export function createMentionNode(absPath: string, isDir?: boolean): HTMLElement {
  const chip = document.createElement("span");
  chip.className = "composer-mention";
  chip.contentEditable = "false";
  chip.dataset.path = absPath;
  chip.setAttribute("title", absPath);

  const icon = document.createElement("span");
  icon.className = "composer-mention-icon";
  icon.innerHTML = mentionIconSvg(absPath, isDir);
  chip.appendChild(icon);

  const label = document.createElement("span");
  label.className = "composer-mention-label";
  label.textContent = mentionBasename(absPath);
  chip.appendChild(label);

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "composer-mention-remove";
  remove.textContent = "×";
  remove.tabIndex = -1;
  chip.appendChild(remove);

  return chip;
}

/** Serialize the editor DOM into plain text (mentions -> `@<path>`). */
export function serializeMentions(root: HTMLElement): string {
  let out = "";
  const walk = (node: Node) => {
    node.childNodes.forEach((child) => {
      if (child.nodeType === Node.TEXT_NODE) {
        out += child.textContent ?? "";
        return;
      }
      if (child.nodeType !== Node.ELEMENT_NODE) return;
      const el = child as HTMLElement;
      if (el.dataset.path) {
        out += serializeMentionPath(el.dataset.path);
      } else if (el.tagName === "BR") {
        out += "\n";
      } else if (el.tagName === "DIV" || el.tagName === "P") {
        if (out && !out.endsWith("\n")) out += "\n";
        walk(el);
      } else {
        walk(el);
      }
    });
  };
  walk(root);
  return out;
}

/** Collect mention paths currently present in the DOM, in document order. */
export function collectMentions(root: HTMLElement): string[] {
  const paths: string[] = [];
  root.querySelectorAll<HTMLElement>(".composer-mention").forEach((el) => {
    if (el.dataset.path) paths.push(el.dataset.path);
  });
  return paths;
}

function appendTextWithBreaks(nodes: Node[], text: string) {
  const parts = text.split("\n");
  parts.forEach((part, idx) => {
    if (idx > 0) nodes.push(document.createElement("br"));
    if (part) nodes.push(document.createTextNode(part));
  });
}

/** Rebuild DOM nodes from plain text, restoring `@<path>` as chips. */
export function buildMentionNodes(text: string, mentions: string[]): Node[] {
  const sorted = Array.from(new Set(mentions)).sort((a, b) => b.length - a.length);
  const nodes: Node[] = [];
  let buffer = "";
  let i = 0;
  const flush = () => {
    if (buffer) {
      appendTextWithBreaks(nodes, buffer);
      buffer = "";
    }
  };
  while (i < text.length) {
    if (text[i] === MENTION_PREFIX) {
      const parsed = parseMentionAt(text, i);
      if (parsed) {
        flush();
        nodes.push(createMentionNode(parsed.path));
        i += parsed.length;
        continue;
      }
      const rest = text.slice(i);
      const hit = sorted.find((p) => p && rest.startsWith(serializeMentionPath(p)));
      if (hit) {
        flush();
        nodes.push(createMentionNode(hit));
        i += serializeMentionPath(hit).length;
        continue;
      }
    }
    buffer += text[i];
    i += 1;
  }
  flush();
  return nodes;
}

export function moveCaretToEnd(root: HTMLElement) {
  const sel = window.getSelection();
  if (!sel) return;
  const range = document.createRange();
  range.selectNodeContents(root);
  range.collapse(false);
  sel.removeAllRanges();
  sel.addRange(range);
}
