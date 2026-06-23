import { useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { useReader, type ReaderFileTab } from "../../store/reader";
import { api } from "../../api/tauri";
import type { PendingDiffLineRange } from "../../utils/inlineDiff";

interface ReaderDiffActionBarProps {
  tab: ReaderFileTab;
  range: PendingDiffLineRange;
  hunkIndex: number;
  hunkTotal: number;
  anchorEl: HTMLDivElement | null;
  mainRef: React.RefObject<HTMLDivElement | null>;
  onNavigate: (direction: -1 | 1) => void;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

export function ReaderDiffActionBar({
  tab,
  range,
  hunkIndex,
  hunkTotal,
  anchorEl,
  mainRef,
  onNavigate,
  onMouseEnter,
  onMouseLeave,
}: ReaderDiffActionBarProps) {
  const { t } = useTranslation();
  const sessionId = useSession((s) => s.activeId);
  const confirmDiffBlock = useReader((s) => s.confirmDiffBlock);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const keepVisible = useCallback(() => {
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    onMouseEnter();
  }, [onMouseEnter]);

  const scheduleHide = useCallback(() => {
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = setTimeout(onMouseLeave, 220);
  }, [onMouseLeave]);

  const onConfirm = async (accept: boolean) => {
    onMouseLeave();
    const result = confirmDiffBlock(tab.path, range.blockId, accept);
    if (!accept && result?.revertText && sessionId) {
      try {
        await api.writeProjectFile(sessionId, tab.path, result.revertText);
      } catch {
        /* still update UI */
      }
    }
  };

  if (!anchorEl || !mainRef.current) return null;

  const mainEl = mainRef.current;

  const anchorRect = anchorEl.getBoundingClientRect();
  const mainRect = mainEl.getBoundingClientRect();
  const top = anchorRect.bottom - mainRect.top + 6;
  const right = mainRect.right - anchorRect.right;

  return (
    <div
      className="reader-diff-actionbar"
      style={{ top, right }}
      onMouseEnter={keepVisible}
      onMouseLeave={scheduleHide}
    >
      <div className="reader-diff-actionbar-nav">
        <button
          type="button"
          className="reader-diff-actionbar-arrow"
          disabled={hunkIndex <= 0}
          aria-label={t("reader.diffPrevHunk")}
          onClick={() => onNavigate(-1)}
        >
          ↑
        </button>
        <span className="reader-diff-actionbar-count">
          {t("reader.diffHunkCount", { current: hunkIndex + 1, total: hunkTotal })}
        </span>
        <button
          type="button"
          className="reader-diff-actionbar-arrow"
          disabled={hunkIndex >= hunkTotal - 1}
          aria-label={t("reader.diffNextHunk")}
          onClick={() => onNavigate(1)}
        >
          ↓
        </button>
      </div>
      <button
        type="button"
        className="reader-diff-actionbar-btn undo"
        onClick={() => void onConfirm(false)}
      >
        {t("reader.diffUndo")}
      </button>
      <button
        type="button"
        className="reader-diff-actionbar-btn keep"
        onClick={() => void onConfirm(true)}
      >
        {t("reader.diffKeep")}
      </button>
    </div>
  );
}
