import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useLayoutEffect, useRef } from "react";
import {
  readerViewportTop,
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
    el.scrollTop = readerViewportTop(path, "preview");
    const onScroll = () => rememberReaderViewport(path, el.scrollTop, "preview");
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      onScroll();
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
