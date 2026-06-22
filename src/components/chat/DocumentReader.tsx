import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useReader, countChars, readerFileName } from "../../store/reader";

export function DocumentReader() {
  const { t } = useTranslation();
  const doc = useReader((s) => s.doc);

  const chars = useMemo(() => {
    if (!doc) return 0;
    return typeof doc.chars === "number" ? doc.chars : countChars(doc.text);
  }, [doc]);

  const lines = useMemo(() => {
    if (!doc) return 0;
    return typeof doc.lines === "number" ? doc.lines : doc.text.split(/\n/).length;
  }, [doc]);

  if (!doc) {
    return (
      <div className="document-reader is-empty">
        <p className="document-reader-empty">{t("rightPanel.readerEmpty")}</p>
      </div>
    );
  }

  const fileName = readerFileName(doc.path);

  return (
    <div className="document-reader">
      <div className="document-reader-head">
        <div className="document-reader-title" title={doc.path}>
          {fileName}
        </div>
        <div className="document-reader-stats">
          <span className="reader-stat">{t("rightPanel.readerChars", { count: chars })}</span>
          <span className="reader-stat">{t("rightPanel.readerLines", { count: lines })}</span>
          {doc.truncated && (
            <span className="reader-stat reader-stat-warn">
              {t("rightPanel.readerTruncated")}
            </span>
          )}
        </div>
      </div>
      <div className="document-reader-body">
        {doc.fileType === "markdown" ? (
          <div className="markdown-body">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{doc.text}</ReactMarkdown>
          </div>
        ) : (
          <pre className="reader-text">{doc.text}</pre>
        )}
      </div>
    </div>
  );
}
