import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  useReader,
  countChars,
  readerFileName,
  type ReaderFileTab,
} from "../../store/reader";
import { ReaderEditor } from "./ReaderEditor";

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <path d="M18 6L6 18M6 6l12 12" />
    </svg>
  );
}

function ReaderFilePane({ tab }: { tab: ReaderFileTab }) {
  const { t } = useTranslation();
  const fileName = readerFileName(tab.path);
  const chars = useMemo(
    () => (typeof tab.chars === "number" ? tab.chars : countChars(tab.text)),
    [tab.chars, tab.text],
  );
  const lines = useMemo(
    () => (typeof tab.lines === "number" ? tab.lines : tab.text.split(/\n/).length),
    [tab.lines, tab.text],
  );

  return (
    <div className="document-reader reader-file-pane">
      <div className="document-reader-head">
        <div className="document-reader-title" title={tab.path}>
          {fileName}
          {tab.dirty && <span className="reader-tab-dirty" title={t("reader.unsaved")} />}
          {tab.saveError && <span className="reader-tab-error" title={t("reader.saveFailed")} />}
        </div>
        <div className="document-reader-stats">
          <span className="reader-stat">{t("rightPanel.readerChars", { count: chars })}</span>
          <span className="reader-stat">{t("rightPanel.readerLines", { count: lines })}</span>
          {tab.truncated && (
            <span className="reader-stat reader-stat-warn">{t("rightPanel.readerTruncated")}</span>
          )}
        </div>
      </div>
      <div className="document-reader-body reader-file-body">
        <ReaderEditor tab={tab} />
      </div>
    </div>
  );
}

export function ReaderWorkspace() {
  const { t } = useTranslation();
  const tabs = useReader((s) => s.tabs);
  const activeTabId = useReader((s) => s.activeTabId);
  const setActiveTab = useReader((s) => s.setActiveTab);
  const closeTab = useReader((s) => s.closeTab);

  const activeTab = tabs.find((tb) => tb.id === activeTabId) ?? null;

  if (tabs.length === 0) {
    return (
      <div className="document-reader is-empty reader-workspace">
        <p className="document-reader-empty">{t("rightPanel.readerEmpty")}</p>
      </div>
    );
  }

  return (
    <div className="reader-workspace">
      <div className="reader-file-tabbar" role="tablist" aria-label={t("reader.fileTabs")}>
        {tabs.map((tb) => {
          const name = readerFileName(tb.path);
          const isActive = tb.id === activeTabId;
          return (
            <div
              key={tb.id}
              className={`reader-file-tab ${isActive ? "is-active" : ""}`}
              role="tab"
              aria-selected={isActive}
            >
              <button
                type="button"
                className="reader-file-tab-label"
                onClick={() => setActiveTab(tb.id)}
                title={tb.path}
              >
                {name}
                {tb.dirty && <span className="reader-file-tab-dot" />}
                {tb.pendingDiffs.length > 0 && <span className="reader-file-tab-diff" />}
              </button>
              <button
                type="button"
                className="reader-file-tab-close"
                title={t("rightPanel.closeTab")}
                onClick={() => closeTab(tb.id)}
              >
                <CloseIcon />
              </button>
            </div>
          );
        })}
      </div>
      {activeTab ? (
        <ReaderFilePane tab={activeTab} />
      ) : (
        <div className="document-reader is-empty">
          <p className="document-reader-empty">{t("rightPanel.readerEmpty")}</p>
        </div>
      )}
    </div>
  );
}

/** @deprecated Use ReaderWorkspace */
export function DocumentReader() {
  return <ReaderWorkspace />;
}
