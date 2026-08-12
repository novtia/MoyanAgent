import { memo, useRef } from "react";

/**
 * Renders a long string as a sequence of independently memoized text nodes.
 *
 * A streaming reply grows by a few characters per frame. Held in one text node
 * that means the browser re-shapes the entire message on every frame, so the
 * cost of each frame scales with how much has been written so far — a 50k-char
 * answer crawls by the end. Splitting the string means only the final node is
 * dirty per frame; the earlier ones keep their cached shaping and line boxes.
 *
 * The chunks are plain text nodes (no wrapper elements), so the parent's
 * `white-space: pre-wrap` still flows them as one continuous run and the
 * rendered output is byte-for-byte what a single node would produce.
 */

/**
 * Large enough that ordinary replies stay a single chunk and pay nothing,
 * small enough that re-shaping the streaming tail stays cheap.
 */
const CHUNK_SIZE = 4000;

/** Don't take a newline boundary that would leave a stubby chunk behind. */
const MIN_CHUNK = 1000;

function splitChunks(text: string): string[] {
  if (text.length <= CHUNK_SIZE) return [text];
  const out: string[] = [];
  let pos = 0;
  while (text.length - pos > CHUNK_SIZE) {
    const hardEnd = pos + CHUNK_SIZE;
    // Cut just after a newline where possible: a boundary that lands mid-line
    // splits a line box across two nodes, which is exactly the case the
    // browser cannot reuse.
    const newline = text.lastIndexOf("\n", hardEnd - 1);
    const cut = newline >= pos + MIN_CHUNK ? newline + 1 : hardEnd;
    out.push(text.slice(pos, cut));
    pos = cut;
  }
  out.push(text.slice(pos));
  return out;
}

interface ChunkCache {
  text: string;
  chunks: string[];
}

function chunksFor(cache: ChunkCache | null, text: string): string[] {
  if (cache && cache.text === text) return cache.chunks;
  // Streaming only ever appends, and the split is a left-to-right greedy scan,
  // so every chunk but the last is already final: re-scan the tail alone
  // instead of walking the whole message again on each frame.
  if (cache && text.length > cache.text.length && text.startsWith(cache.text)) {
    const sealed = cache.chunks.slice(0, -1);
    const tail = cache.chunks[cache.chunks.length - 1];
    return sealed.concat(splitChunks(text.slice(cache.text.length - tail.length)));
  }
  return splitChunks(text);
}

const TextChunk = memo(function TextChunk({ text }: { text: string }) {
  return <>{text}</>;
});

export function ChunkedText({ text }: { text: string }) {
  // Render-time cache. `chunksFor` is pure and deterministic, so a double
  // invocation under StrictMode produces the same chunks.
  const cache = useRef<ChunkCache | null>(null);
  const chunks = chunksFor(cache.current, text);
  cache.current = { text, chunks };

  return (
    <>
      {chunks.map((chunk, i) => (
        <TextChunk key={i} text={chunk} />
      ))}
    </>
  );
}
