import type { MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { ReaderFileTab } from "../../../../../store/reader";
import {
  BackIcon,
  EyeIcon,
  FilesIcon,
  ForwardIcon,
  IndentIcon,
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
  /** Media tabs hide Preview/Source and disable file-scoped find. */
  isMedia: boolean;
  hasFile: boolean;
  findOpen: boolean;
  showTree: boolean;
  rightViewIsTree: boolean;
  canIndent: boolean;
  indentTitle: string;
  onPreview: () => void;
  onSource: () => void;
  onIndent: (e: ReactMouseEvent) => void;
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
  isMedia,
  hasFile,
  findOpen,
  showTree,
  rightViewIsTree,
  canIndent,
  indentTitle,
  onPreview,
  onSource,
  onIndent,
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
        {!isMedia && tab != null && (
          <span className="reader-toolbar-chars">
            · {t("rightPanel.readerChars", { count: tab.chars ?? 0 })}
          </span>
        )}
        {tab?.dirty && <span className="reader-tab-dirty" title={t("reader.saveHint")} />}
        {tab?.saveError && <span className="reader-tab-error" title={t("reader.saveFailed")} />}
      </div>
      <div className="reader-toolbar-actions">
        {!isMedia && (
          <>
            <button
              type="button"
              className={`reader-toolbar-btn${preview && isMarkdown ? " is-active" : ""}`}
              title={t("reader.preview")}
              disabled={!isMarkdown}
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
          </>
        )}
        {!isMedia && (
          <button
            type="button"
            className="reader-toolbar-btn"
            title={indentTitle}
            aria-label={t("reader.indentFirstLine")}
            disabled={!canIndent}
            onClick={onIndent}
          >
            <IndentIcon />
          </button>
        )}
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
          title={isMedia ? t("reader.mediaFindDisabled") : t("reader.search")}
          aria-pressed={findOpen}
          disabled={isMedia}
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
