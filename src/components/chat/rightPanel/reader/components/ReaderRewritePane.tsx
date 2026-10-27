import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ThinkingBlock } from "../../../messageList/ThinkingBlock";
import { copyText } from "../../../../../utils/clipboard";
import { toast } from "../../../../ui/Toast";
import type { RewriteRun } from "../../../../../store/readerRewrite";

export function ReaderRewritePane({ run }: { run: RewriteRun }) {
  const { t } = useTranslation();
  const bodyRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [run.text, run.running]);

  const stageLabel =
    run.stage === "expand" ? t("reader.expandMode") : t("reader.rewriteMode");
  const modeLabel =
    run.mode === "expand"
      ? t("reader.expandMode")
      : run.mode === "pipeline"
        ? t("reader.pipelineMode")
        : t("reader.rewriteMode");
  const progress =
    run.total > 0
      ? t("reader.rewriteProgress", {
          stage: stageLabel,
          index: run.index,
          total: run.total,
        })
      : t("reader.rewriteWorking");

  return (
    <div className="reader-rewrite-pane">
      <div className="reader-rewrite-head">
        <span className="reader-rewrite-progress">
          {run.running ? progress : run.cancelled ? t("reader.rewriteStopped") : modeLabel}
        </span>
        <button
          type="button"
          className="reader-rewrite-copy"
          disabled={!run.text}
          onClick={() => {
            void copyText(run.text).then(() => toast.success(t("reader.rewriteCopied")));
          }}
        >
          {t("reader.rewriteCopy")}
        </button>
      </div>
      {run.error && <div className="reader-rewrite-error">{run.error}</div>}
      {run.thinking && (
        <ThinkingBlock content={run.thinking} streaming={run.running} />
      )}
      <div ref={bodyRef} className="reader-rewrite-body">
        {run.text || (run.running ? "" : t("reader.rewritePlaceholder"))}
      </div>
    </div>
  );
}
