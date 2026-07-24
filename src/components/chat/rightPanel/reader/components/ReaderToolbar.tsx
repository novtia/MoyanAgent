import type { MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { ReaderFileTab } from "../../../../../store/reader";
import {
  BackIcon,
  EyeIcon,
  FilesIcon,
  ForwardIcon,
  MoreIcon,
  SearchIcon,
  SourceIcon,
} from "../icons";

export interface ReaderToolbarProps {
  path: string | null | undefined;
  fileName: string;
  tab: ReaderFileTab | null;
  canBack: boolean;
  canForward: boolean;
  onBack: () => void;
  onForward: () => void;
  preview: boolean;
  isMarkdown: boolean;
  hasPendingDiff: boolean;
  hasFile: boolean;
  findOpen: boolean;
  showTree: boolean;
  rightViewIsTree: boolean;
  onPreview: () => void;
  onSource: () => void;
  onMore: (e: ReactMouseEvent) => void;
  onToggleSearch: () => void;
  onToggleFileTree: () => void;
}

export function ReaderToolbar({
  path,
  fileName,
  tab,
  canBack,
  canForward,
  onBack,
  onForward,
  preview,
  isMarkdown,
  hasPendingDiff,
  hasFile,
  findOpen,
  showTree,
  rightViewIsTree,
  onPreview,
  onSource,
  onMore,
  onToggleSearch,
  onToggleFileTree,
}: ReaderToolbarProps) {
  const { t } = useTranslation();

  return (
    <div className="reader-toolbar">
      <div className="reader-toolbar-nav">
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.back")}
          disabled={!canBack}
          onClick={onBack}
        >
          <BackIcon />
        </button>
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.forward")}
          disabled={!canForward}
          onClick={onForward}
        >
          <ForwardIcon />
        </button>
      </div>
      <div className="reader-toolbar-title" title={path ?? undefined}>
        <span className="reader-toolbar-name">{fileName || t("rightPanel.readerTab")}</span>
        {tab?.dirty && <span className="reader-tab-dirty" title={t("reader.unsaved")} />}
        {tab?.saveError && <span className="reader-tab-error" title={t("reader.saveFailed")} />}
      </div>
      <div className="reader-toolbar-actions">
        <button
          type="button"
          className={`reader-toolbar-btn${preview && isMarkdown ? " is-active" : ""}`}
          title={t("reader.preview")}
          disabled={!isMarkdown || hasPendingDiff}
          onClick={onPreview}
        >
          <EyeIcon />
        </button>
        <button
          type="button"
          className={`reader-toolbar-btn${!preview ? " is-active" : ""}`}
          title={t("reader.source")}
          disabled={!hasFile}
          onClick={onSource}
        >
          <SourceIcon />
        </button>
        <button
          type="button"
          className="reader-toolbar-btn"
          title={t("reader.more")}
          disabled={!hasFile}
          onClick={onMore}
        >
          <MoreIcon />
        </button>
        <span className="reader-toolbar-sep" aria-hidden />
        <button
          type="button"
          className={`reader-toolbar-btn${findOpen ? " is-active" : ""}`}
          title={t("reader.search")}
          aria-pressed={findOpen}
          onClick={onToggleSearch}
        >
          <SearchIcon />
        </button>
        <button
          type="button"
          className={`reader-toolbar-btn${showTree && rightViewIsTree ? " is-active" : ""}`}
          title={showTree && rightViewIsTree ? t("reader.hideFiles") : t("reader.showFiles")}
          aria-pressed={showTree && rightViewIsTree}
          onClick={onToggleFileTree}
        >
          <FilesIcon />
        </button>
      </div>
    </div>
  );
}
