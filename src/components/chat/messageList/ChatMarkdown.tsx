import { memo, useMemo, type ComponentProps } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../../../api/tauri";

/**
 * Markdown renderer for assistant chat text.
 *
 * A streaming reply is re-rendered on every frame. Re-parsing the whole message
 * each time makes the cost grow with the message length, so the text is split
 * into top-level blocks and each block is memoized on its source string:
 * once a block is complete, later frames reuse it and only the tail re-parses.
 */

const REMARK_PLUGINS = [remarkGfm];

const FENCE_RE = /^ {0,3}(`{3,}|~{3,})/;
const LIST_ITEM_RE = /^(?:[-*+]|\d{1,9}[.)])(?:\s|$)/;

/**
 * Split markdown into top-level blocks at blank lines that are safe to cut:
 * outside code fences, and followed by an unindented line that is neither a
 * list item nor a blockquote (those can continue the previous block).
 */
export function splitMarkdownBlocks(text: string): string[] {
  const lines = text.split("\n");
  const blocks: string[] = [];
  let current: string[] = [];
  let fenceChar = "";
  let fenceLen = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const m = FENCE_RE.exec(line);
    if (m) {
      const marker = m[1];
      if (!fenceChar) {
        fenceChar = marker[0];
        fenceLen = marker.length;
      } else if (marker[0] === fenceChar && marker.length >= fenceLen) {
        fenceChar = "";
        fenceLen = 0;
      }
    }

    current.push(line);

    if (fenceChar || line.trim() !== "") continue;

    // `line` is blank: look ahead to the next non-blank line.
    let j = i + 1;
    while (j < lines.length && lines[j].trim() === "") j++;
    if (j >= lines.length) continue;
    const next = lines[j];
    const unsafe =
      /^\s/.test(next) || next.startsWith(">") || LIST_ITEM_RE.test(next);
    if (unsafe) continue;

    // Cut after the blank run so it stays with the finished block.
    for (let k = i + 1; k < j; k++) current.push(lines[k]);
    i = j - 1;
    blocks.push(current.join("\n"));
    current = [];
  }

  if (current.length > 0) blocks.push(current.join("\n"));
  return blocks;
}

const COMPONENTS: Components = {
  a({ href, children, node: _node, ...rest }: ComponentProps<"a"> & { node?: unknown }) {
    return (
      <a
        {...rest}
        href={href}
        target="_blank"
        rel="noreferrer"
        onClick={(e) => {
          // Never navigate the app webview itself.
          e.preventDefault();
          if (href && /^https?:\/\//i.test(href)) {
            api.openUrl(href).catch(console.warn);
          }
        }}
      >
        {children}
      </a>
    );
  },
  table({ children, node: _node, ...rest }: ComponentProps<"table"> & { node?: unknown }) {
    return (
      <div className="chat-md-table-wrap">
        <table {...rest}>{children}</table>
      </div>
    );
  },
};

const MarkdownBlock = memo(function MarkdownBlock({ text }: { text: string }) {
  return (
    <Markdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
      {text}
    </Markdown>
  );
});

export function ChatMarkdown({ text }: { text: string }) {
  const blocks = useMemo(() => splitMarkdownBlocks(text), [text]);
  return (
    <>
      {blocks.map((block, i) => (
        <MarkdownBlock key={i} text={block} />
      ))}
    </>
  );
}
