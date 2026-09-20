import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, srcOf } from "../../../api/tauri";
import type { AssistantBlock } from "../../../types";
import { parseNovelAiOutput } from "./parsers";
import { ToolGlyph } from "./toolIcons";
import { extractToolErrorMessage } from "./utils";

export function NovelAIToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(true);
  const status = block.status;
  const input = (block.input ?? {}) as { prompt?: string; title?: string };
  const parsed = useMemo(
    () => parseNovelAiOutput(block.output),
    [block.output],
  );
  const prompt = parsed?.prompt || input.prompt || "";
  const size =
    parsed?.width && parsed?.height
      ? `${parsed.width}×${parsed.height}`
      : "";
  const meta = [
    parsed?.model,
    size,
    parsed?.seed != null ? `seed ${parsed.seed}` : "",
    input.title,
  ]
    .filter(Boolean)
    .join(" · ");
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";
  const images = parsed?.images ?? [];
  const hasDetail =
    !!errorMessage || images.length > 0 || (status === "success" && !!prompt);

  return (
    <div className={`nai-tool${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="flow-head"
        aria-expanded={hasDetail ? open : undefined}
        title={hasDetail ? t("message.toolCallToggle") : undefined}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <span className="ti">
          <ToolGlyph tool="NovelAI" />
        </span>
        <span className="t">NovelAI</span>
        {meta && <span className="m">{meta}</span>}
      </button>
      {open && hasDetail && (
        <div className="nai-tool-body">
          {errorMessage && (
            <div className="tool-call-error-detail" role="alert">
              <span className="tool-call-error-detail-text">{errorMessage}</span>
            </div>
          )}
          {images.length > 0 && (
            <div className="nai-tool-thumbs">
              {images.map((img) => (
                <button
                  key={img.path}
                  type="button"
                  className="nai-tool-thumb"
                  title={img.path}
                  onClick={() => {
                    api.openPath(img.path).catch(console.warn);
                  }}
                >
                  <img src={srcOf(img.path)} alt="" />
                </button>
              ))}
            </div>
          )}
          {prompt && (
            <div className="nai-tool-prompt" title={prompt}>
              {prompt}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
