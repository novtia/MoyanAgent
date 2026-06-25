import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import { useReader, type ReaderFileTab } from "../../store/reader";
import { api } from "../../api/tauri";

interface ReaderDiffHeaderBarProps {
  tab: ReaderFileTab;
  activeIndex: number;
  onNavigate: (direction: -1 | 1) => void;
  onAcceptAll: () => void;
  onRejectAll: () => void;
}

export function ReaderDiffHeaderBar({
  tab,
  activeIndex,
  onNavigate,
  onAcceptAll,
  onRejectAll,
}: ReaderDiffHeaderBarProps) {
  const { t } = useTranslation();
  const sessionId = useSession((s) => s.activeId);
  const confirmAllDiffs = useReader((s) => s.confirmAllDiffs);
  const rejectAllDiffs = useReader((s) => s.rejectAllDiffs);

  const hunkTotal = tab.pendingDiffs.length;

  const safeIndex = hunkTotal > 0 ? Math.min(Math.max(activeIndex, 0), hunkTotal - 1) : 0;

  const handleAcceptAll = () => {
    confirmAllDiffs(tab.path);
    onAcceptAll();
  };

  const handleRejectAll = async () => {
    const result = rejectAllDiffs(tab.path);
    if (result?.revertText && sessionId) {
      try {
        await api.writeProjectFile(sessionId, tab.path, result.revertText);
      } catch {
        /* still update UI */
      }
    }
    onRejectAll();
  };

  if (hunkTotal === 0) return null;

  return (
    <div className="reader-diff-header-bar">
      <div className="reader-diff-actionbar-nav">
        <button
          type="button"
          className="reader-diff-actionbar-arrow"
          disabled={safeIndex <= 0}
          aria-label={t("reader.diffPrevHunk")}
          onClick={() => onNavigate(-1)}
        >
          ↑
        </button>
        <span className="reader-diff-actionbar-count">
          {t("reader.diffHunkCount", { current: safeIndex + 1, total: hunkTotal })}
        </span>
        <button
          type="button"
          className="reader-diff-actionbar-arrow"
          disabled={safeIndex >= hunkTotal - 1}
          aria-label={t("reader.diffNextHunk")}
          onClick={() => onNavigate(1)}
        >
          ↓
        </button>
      </div>
      <button
        type="button"
        className="reader-diff-actionbar-btn undo"
        onClick={() => void handleRejectAll()}
      >
        {t("reader.diffRejectAll")}
      </button>
      <button
        type="button"
        className="reader-diff-actionbar-btn keep"
        onClick={handleAcceptAll}
      >
        {t("reader.diffAcceptAll")}
      </button>
    </div>
  );
}
