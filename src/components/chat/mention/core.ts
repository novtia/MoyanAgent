/**
 * Single source of truth for `@file` mention "reference cards".
 *
 * All mention logic — path normalization, icon selection, serialization to/from
 * the `@<path>` text form, the contenteditable chip DOM node, and project-scope
 * validation — lives here so the editor, the static renderer and the drop
 * handlers all behave identically. No React in this file (pure + DOM helpers).
 */

export const MENTION_PREFIX = "@";

/**
 * Matches a serialized mention in plain text: `@` followed by an absolute path
 * (Windows drive `C:\`, UNC `\\`, or POSIX `/`), spanning until the next
 * whitespace or `@`.
 */
export const MENTION_RE = /@((?:[A-Za-z]:[\\/]|\\\\|\/)[^\s@]*)/g;

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

/** Extract the mention paths from serialized text, in document order. */
export function parseMentionPaths(text: string): string[] {
  const paths: string[] = [];
  if (!text) return paths;
  MENTION_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = MENTION_RE.exec(text)) !== null) paths.push(m[1]);
  return paths;
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
export function createMentionNode(absPath: string, _isDir?: boolean): HTMLElement {
  const chip = document.createElement("span");
  chip.className = "composer-mention";
  chip.contentEditable = "false";
  chip.dataset.path = absPath;
  chip.setAttribute("title", absPath);

  // A real text glyph as the FIRST child gives the inline-flex chip a genuine
  // text baseline (an SVG icon would not), so the label aligns with the
  // surrounding text's baseline. See `.composer-mention` in composer.css.
  const at = document.createElement("span");
  at.className = "composer-mention-at";
  at.textContent = MENTION_PREFIX;
  chip.appendChild(at);

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
        out += `${MENTION_PREFIX}${el.dataset.path}`;
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
      const rest = text.slice(i + 1);
      const hit = sorted.find((p) => p && rest.startsWith(p));
      if (hit) {
        flush();
        nodes.push(createMentionNode(hit));
        i += 1 + hit.length;
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
