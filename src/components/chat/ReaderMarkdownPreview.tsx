import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

/** Rendered markdown preview for the reader's Preview mode. */
export function ReaderMarkdownPreview({ text }: { text: string }) {
  return (
    <div className="reader-md-preview">
      <div className="reader-md-preview-inner">
        <Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>
      </div>
    </div>
  );
}
