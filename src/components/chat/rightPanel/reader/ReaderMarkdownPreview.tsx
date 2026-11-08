import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useLayoutEffect, useRef } from "react";
import {
  flushReaderProgress,
  hasReaderViewport,
  readerSavedProgress,
  readerViewportTop,
  rememberReaderRatio,
  rememberReaderViewport,
} from "../../../../utils/readerViewport";

/** Shared markdown renderer (reader preview + skill detail, etc.). */
export function ReaderMarkdownPreview({
  text,
  className = "reader-md-preview",
  path,
}: {
  text: string;
  /** Outer wrapper class; defaults to reader scroll pane chrome. */
  className?: string;
  /** When set, scroll position is remembered across remounts / agent edits. */
  path?: string;
}) {
  const scrollerRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el || !path) return;
    const maxScroll = () => Math.max(0, el.scrollHeight - el.clientHeight);
    let touched = false;
    if (hasReaderViewport(path, "preview")) {
      el.scrollTop = readerViewportTop(path, "preview");
    } else {
      // Fresh start / different layout: restore by reading fraction instead.
      const saved = readerSavedProgress(path);
      if (saved && saved.ratio > 0) el.scrollTop = saved.ratio * maxScroll();
    }
    const onScroll = () => {
      touched = true;
      rememberReaderViewport(path, el.scrollTop, "preview");
      const max = maxScroll();
      rememberReaderRatio(path, max > 0 ? el.scrollTop / max : 0);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      // Never overwrite saved progress with a position the user never set.
      if (touched) onScroll();
      else if (hasReaderViewport(path, "preview")) {
        rememberReaderViewport(path, el.scrollTop, "preview");
      }
      flushReaderProgress();
      el.removeEventListener("scroll", onScroll);
    };
  }, [path]);

  // Agent edits rewrite inner markdown; restore so the browser cannot snap to top.
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el || !path) return;
    const saved = readerViewportTop(path, "preview");
    if (Math.abs(el.scrollTop - saved) > 1) el.scrollTop = saved;
  }, [path, text]);

  return (
    <div ref={scrollerRef} className={className}>
      <div className="reader-md-preview-inner">
        <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
      </div>
    </div>
  );
}
