import { useTranslation } from "react-i18next";
import { useSession } from "../../../../store/session";
import { useReader, type ReaderFileTab } from "../../../../store/reader";
import { api } from "../../../../api/tauri";
import type { PendingDiffLineRange } from "../../../../utils/inlineDiff";

interface ReaderDiffActionBarProps {
  tab: ReaderFileTab;
  range: PendingDiffLineRange;
  hunkIndex: number;
  hunkTotal: number;
  onNavigate: (direction: -1 | 1) => void;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

export function ReaderDiffActionBar({
  tab,
  range,
  hunkIndex,
  hunkTotal,
  onNavigate,
  onMouseEnter,
  onMouseLeave,
}: ReaderDiffActionBarProps) {
  const { t } = useTranslation();
  const sessionId = useSession((s) => s.activeId);
  const confirmDiffBlock = useReader((s) => s.confirmDiffBlock);

  const onConfirm = async (accept: boolean) => {
    onMouseLeave();
    if (!sessionId) return;
    try {
      const revert = await api.confirmPendingDiff(sessionId, range.blockId, accept);
      if (accept) {
        confirmDiffBlock(tab.path, range.blockId, true);
        return;
      }
      if (!revert) return;
      const result = confirmDiffBlock(tab.path, range.blockId, false);
      if (result?.revertText !== revert.text) {
        useReader.getState().updateTabText(tab.path, revert.text, { dirty: false });
      }
    } catch {
      /* keep UI pending on failure */
    }
  };

  return (
    <div
      className="reader-diff-actionbar"
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      <div className="reader-diff-actionbar-nav">
        <button
          type="button"
          className="reader-diff-actionbar-arrow"
          disabled={hunkIndex <= 0}
          aria-label={t("reader.diffPrevHunk")}
          onClick={() => onNavigate(-1)}
        >
          {"\u2190"}
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
          {"\u2192"}
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
