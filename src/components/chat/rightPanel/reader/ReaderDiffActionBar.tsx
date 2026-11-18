import { useTranslation } from "react-i18next";
import { useSession } from "../../../../store/session";
import { useReader, syncPendingDiffsForPath, type ReaderFileTab } from "../../../../store/reader";
import { api } from "../../../../api/tauri";
import type { PendingDiffLineRange } from "../../../../utils/inlineDiff";

interface ReaderDiffActionBarProps {
  tab: ReaderFileTab;
  range: PendingDiffLineRange;
  /** Every Edit merged into this hunk (chronological); defaults to just `range`. */
  blockIds?: string[];
  hunkIndex: number;
  hunkTotal: number;
  onNavigate: (direction: -1 | 1) => void;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

export function ReaderDiffActionBar({
  tab,
  range,
  blockIds,
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
      // A merged hunk stands for several Edits; Keep/Undo acts on all of them
      // but never on other hunks. The backend rewrites the remaining rows, so
      // re-sync from it afterwards.
      const ids = blockIds ?? [range.blockId];
      const hasLocalOnly = ids.some((id) => id.startsWith("local-rewrite-"));
      if (hasLocalOnly) {
        if (accept) {
          for (const id of ids) confirmDiffBlock(tab.path, id, true);
        } else {
          let revertText: string | null = null;
          for (const id of [...ids].reverse()) {
            const res = confirmDiffBlock(tab.path, id, false);
            if (res?.revertText != null) revertText = res.revertText;
          }
          if (revertText != null) {
            await api.writeProjectFile(sessionId, tab.path, revertText, tab.encoding, tab.hadBom);
          }
        }
        return;
      }
      if (accept) {
        for (const id of ids) {
          await api.confirmPendingDiff(sessionId, id, true);
          confirmDiffBlock(tab.path, id, true);
        }
      } else {
        let revert: Awaited<ReturnType<typeof api.confirmPendingDiff>> = null;
        for (const id of [...ids].reverse()) {
          revert = (await api.confirmPendingDiff(sessionId, id, false)) ?? revert;
        }
        if (revert) {
          useReader.getState().updateTabText(tab.path, revert.text, { dirty: false });
        }
      }
      await syncPendingDiffsForPath(sessionId, tab.path);
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
