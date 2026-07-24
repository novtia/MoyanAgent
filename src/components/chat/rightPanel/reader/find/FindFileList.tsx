import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { normalizeReaderPath } from "../../../../../store/reader";
import type { ReaderFindMatch } from "../../../../../store/readerFind";

export function FindFileList({
  fileSummaries,
  activeMatch,
  disabled,
  activeListBtnRef,
  onGoToFile,
}: {
  fileSummaries: Array<{ path: string; name: string; count: number }>;
  activeMatch: ReaderFindMatch | null;
  disabled?: boolean;
  activeListBtnRef: RefObject<HTMLButtonElement | null>;
  onGoToFile: (path: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="reader-find-file-panel">
      <div className="reader-find-file-panel-head">
        {t("readerFind.fileList")}
        {fileSummaries.length > 0 && (
          <span className="reader-find-file-panel-meta">
            {t("readerFind.fileMatchCount", { count: fileSummaries.length })}
          </span>
        )}
      </div>
      {fileSummaries.length === 0 ? (
        <p className="reader-find-file-empty">{t("readerFind.noResults")}</p>
      ) : (
        <ul className="reader-find-file-list" role="listbox" aria-label={t("readerFind.fileList")}>
          {fileSummaries.map((file) => {
            const isActive =
              activeMatch != null &&
              normalizeReaderPath(activeMatch.path) === normalizeReaderPath(file.path);
            return (
              <li key={file.path} role="presentation">
                <button
                  ref={isActive ? (activeListBtnRef as React.RefObject<HTMLButtonElement>) : undefined}
                  type="button"
                  role="option"
                  aria-selected={isActive}
                  className={`reader-find-file-item${isActive ? " is-active" : ""}`}
                  title={file.path}
                  onClick={() => onGoToFile(file.path)}
                  disabled={disabled}
                >
                  <span className="reader-find-file-name">{file.name}</span>
                  <span className="reader-find-file-count">
                    {t("readerFind.fileMatchCount", { count: file.count })}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
