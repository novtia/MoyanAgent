import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { sanitizeFsPath } from "../../../utils/sanitizePath";
import { parseGrepOutput } from "./parsers";
import { ToolGlyph } from "./toolIcons";
import { extractToolErrorMessage } from "./utils";

function fileName(path: string): string {
  if (!path) return "";
  const cleaned = sanitizeFsPath(path);
  const parts = cleaned.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || cleaned;
}

/** Highlight every occurrence of any of 	erms (earliest, then longest, wins). */
function highlightQuery(text: string, terms: string[]): ReactNode {
  const needles = terms.filter(Boolean).map((q) => q.toLowerCase());
  if (!needles.length) return text;
  const lower = text.toLowerCase();
  const parts: ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < text.length) {
    let best = -1;
    let bestLen = 0;
    for (const q of needles) {
      const idx = lower.indexOf(q, i);
      if (idx < 0) continue;
      if (best < 0 || idx < best || (idx === best && q.length > bestLen)) {
        best = idx;
        bestLen = q.length;
      }
    }
    if (best < 0) {
      parts.push(text.slice(i));
      break;
    }
    if (best > i) parts.push(text.slice(i, best));
    parts.push(<mark key={key++}>{text.slice(best, best + bestLen)}</mark>);
    i = best + bestLen;
  }
  return parts;
}

export function GrepToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const input = (block.input ?? {}) as {
    // Both are lists; a bare string is tolerated.
    query?: string | string[];
    path?: string | string[];
  };
  const parsed = useMemo(() => parseGrepOutput(block.output), [block.output]);
  const terms = useMemo(() => {
    if (parsed?.queries?.length) return parsed.queries;
    const fromInput = ([] as unknown[])
      .concat(input.query ?? [])
      .filter((q): q is string => typeof q === "string" && !!q);
    if (fromInput.length) return [...new Set(fromInput)];
    return parsed?.query ? [parsed.query] : [];
  }, [parsed, input.query]);
  const query = terms.join(" | ");
  const inputPath = ([] as unknown[]).concat(input.path ?? [])[0];
  const path = sanitizeFsPath(
    parsed?.path || (typeof inputPath === "string" ? inputPath : ""),
  );
  const total =
    parsed?.total_matches ??
    parsed?.files.reduce((n, f) => n + f.matches.length, 0) ??
    0;

  const fileNames = useMemo(() => {
    if (!parsed?.files.length) {
      const name = fileName(path);
      return name ? [name] : [];
    }
    const names = parsed.files.map((f) => fileName(f.path)).filter(Boolean);
    return [...new Set(names)];
  }, [parsed, path]);

  const meta = [
    query ? `"${query}"` : "",
    fileNames.length === 1
      ? fileNames[0]
      : fileNames.length > 1
        ? t("message.grepFileCount", { count: fileNames.length })
        : "",
    status === "success"
      ? t("message.grepMatchCount", { count: total })
      : "",
    parsed?.truncated || parsed?.files_capped
      ? t("message.toolOutputTruncated")
      : "",
  ]
    .filter(Boolean)
    .join(" · ");

  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";
  const hasDetail =
    !!errorMessage ||
    (!!parsed && parsed.files.length > 0) ||
    (status === "success" && !!parsed && total === 0);

  return (
    <div className={`grep-tool${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="flow-head"
        aria-expanded={hasDetail ? open : undefined}
        title={hasDetail ? t("message.toolCallToggle") : undefined}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <span className="ti">
          <ToolGlyph tool="Grep" />
        </span>
        <span className="t">Grep</span>
        {meta && <span className="m">{meta}</span>}
      </button>
      {open && errorMessage && (
        <div className="tool-call-error-detail" role="alert">
          <span className="tool-call-error-detail-text">{errorMessage}</span>
        </div>
      )}
      {open && parsed && parsed.files.length > 0 && (
        <div className="grep">
          {parsed.files.map((file) => {
            const cleanPath = sanitizeFsPath(file.path);
            const name = fileName(cleanPath);
            return (
              <div key={cleanPath}>
                {name && parsed.files.length > 1 && (
                  <div className="grep-file" title={cleanPath}>
                    {name}
                  </div>
                )}
                {file.matches.map((m, i) => (
                  <div className="grep-hit" key={`${cleanPath}:${i}`}>
                    <span className="grep-ln">
                      {m.label ||
                        (m.paragraph != null
                          ? `P${String(m.paragraph).padStart(3, "0")}`
                          : "·")}
                    </span>
                    <span className="grep-text">
                      {highlightQuery(m.text, terms)}
                    </span>
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      )}
      {open && status === "success" && parsed && total === 0 && (
        <div className="kv" style={{ padding: "4px 2px" }}>
          {t("message.grepEmpty")}
        </div>
      )}
    </div>
  );
}
