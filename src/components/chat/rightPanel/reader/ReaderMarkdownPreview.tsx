import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

/** Shared markdown renderer (reader preview + skill detail, etc.). */
export function ReaderMarkdownPreview({
  text,
  className = "reader-md-preview",
}: {
  text: string;
  /** Outer wrapper class; defaults to reader scroll pane chrome. */
  className?: string;
}) {
  return (
    <div className={className}>
      <div className="reader-md-preview-inner">
        <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
      </div>
    </div>
  );
}
