import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SegmentButton } from "../../SegmentButton";
import {
  endpointWithVertexServiceAccountProject,
  looksLikeVertexServiceAccountDraft,
  parseVertexServiceAccount,
} from "../modelServices";
import type { ProviderDraft } from "./types";

type AuthMode = "key" | "json";

export function VertexCredentialFields({
  providerId,
  apiKey,
  endpoint,
  showKey,
  apiKeyPlaceholder,
  validationError,
  onDraftChange,
  onToggleShowKey,
}: {
  providerId: string;
  apiKey: string;
  endpoint: string;
  showKey: boolean;
  apiKeyPlaceholder: string;
  validationError?: string;
  onDraftChange: (patch: Partial<ProviderDraft>) => void;
  onToggleShowKey: () => void;
}) {
  const { t } = useTranslation();
  const fileRef = useRef<HTMLInputElement>(null);
  const [pin, setPin] = useState<{ id: string; mode: AuthMode } | null>(null);
  const [importError, setImportError] = useState<{
    id: string;
    message: string;
  } | null>(null);
  const [pasteDraft, setPasteDraft] = useState<{ id: string; text: string }>({
    id: "",
    text: "",
  });

  const detected: AuthMode = looksLikeVertexServiceAccountDraft(apiKey)
    ? "json"
    : "key";
  const mode = pin?.id === providerId ? pin.mode : detected;
  const parsed = parseVertexServiceAccount(apiKey);
  const draftIsJson = looksLikeVertexServiceAccountDraft(apiKey);
  const showingStored = (mode === "json") === draftIsJson;
  const storedValid = parsed.ok && showingStored;
  const repairingInvalid =
    mode === "json" && showingStored && draftIsJson && !parsed.ok;
  const pasteText = pasteDraft.id === providerId ? pasteDraft.text : "";
  const fileError = importError?.id === providerId ? importError.message : "";

  function setMode(next: AuthMode) {
    setPin({ id: providerId, mode: next });
    setImportError(null);
  }

  function commitJson(text: string): boolean {
    const result = parseVertexServiceAccount(text);
    if (!result.ok) {
      setImportError({ id: providerId, message: result.error });
      return false;
    }
    setImportError(null);
    setPasteDraft({ id: providerId, text: "" });
    setPin({ id: providerId, mode: "json" });
    onDraftChange({
      api_key: result.json,
      endpoint: endpointWithVertexServiceAccountProject(endpoint, result.projectId),
    });
    return true;
  }

  async function onPickFile(file: File | undefined) {
    if (!file) return;
    let text: string;
    try {
      text = await file.text();
    } catch {
      setImportError({
        id: providerId,
        message: t("settings.llm.vertexJsonReadFailed"),
      });
      return;
    }
    commitJson(text);
  }

  function onPasteChange(value: string) {
    if (repairingInvalid) {
      const result = parseVertexServiceAccount(value);
      setImportError(null);
      if (result.ok) {
        setPasteDraft({ id: providerId, text: "" });
        onDraftChange({
          api_key: result.json,
          endpoint: endpointWithVertexServiceAccountProject(
            endpoint,
            result.projectId,
          ),
        });
        return;
      }
      onDraftChange({ api_key: value });
      return;
    }
    setPasteDraft({ id: providerId, text: value });
    const trimmed = value.trim();
    if (!trimmed) {
      setImportError(null);
      return;
    }
    const result = parseVertexServiceAccount(value);
    if (result.ok) {
      commitJson(value);
      return;
    }
    if (trimmed.startsWith("{") && trimmed.endsWith("}")) {
      setImportError({ id: providerId, message: result.error });
    }
  }

  function onPasteBlur(value: string) {
    if (!value.trim()) return;
    const result = parseVertexServiceAccount(value);
    if (!result.ok) setImportError({ id: providerId, message: result.error });
  }

  const error = fileError || (showingStored ? validationError : "") || "";
  let hint = t("settings.llm.vertexKeyHint");
  if (mode === "json") {
    if (!showingStored) hint = t("settings.llm.vertexAuthPendingJson");
    else if (parsed.ok) {
      hint = t("settings.llm.vertexJsonReady", { email: parsed.clientEmail });
    } else hint = t("settings.llm.vertexJsonHint");
  } else if (!showingStored) {
    hint = t("settings.llm.vertexAuthPendingKey");
  }

  return (
    <div className="row">
      <div className="model-provider-switch-head">
        <label className="field-label">{t("settings.llm.vertexAuthLabel")}</label>
        <div
          className="settings-segment"
          role="tablist"
          aria-label={t("settings.llm.vertexAuthLabel")}
        >
          <SegmentButton active={mode === "key"} onClick={() => setMode("key")}>
            {t("settings.llm.vertexAuthModeKey")}
          </SegmentButton>
          <SegmentButton active={mode === "json"} onClick={() => setMode("json")}>
            {t("settings.llm.vertexAuthModeJson")}
          </SegmentButton>
        </div>
      </div>

      {mode === "key" ? (
        <div className="input-affix">
          <input
            type={showKey ? "text" : "password"}
            value={showingStored ? apiKey : ""}
            autoComplete="off"
            spellCheck={false}
            placeholder={apiKeyPlaceholder}
            onChange={(e) => onDraftChange({ api_key: e.target.value })}
          />
          <button type="button" className="affix-btn" onClick={onToggleShowKey}>
            {showKey ? t("settings.llm.keyHide") : t("settings.llm.keyShow")}
          </button>
        </div>
      ) : (
        <>
          <div className="local-server-actions">
            <button
              type="button"
              className="btn primary"
              onClick={() => fileRef.current?.click()}
            >
              {storedValid
                ? t("settings.llm.vertexJsonReimport")
                : t("settings.llm.vertexJsonImport")}
            </button>
            {storedValid ? (
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setImportError(null);
                  setPasteDraft({ id: providerId, text: "" });
                  onDraftChange({ api_key: "" });
                }}
              >
                {t("settings.llm.vertexJsonClear")}
              </button>
            ) : null}
            <input
              ref={fileRef}
              type="file"
              accept=".json,application/json"
              hidden
              onChange={(e) => {
                const file = e.target.files?.[0];
                e.target.value = "";
                void onPickFile(file);
              }}
            />
          </div>
          <label className="field-label vertex-json-paste-label">
            {t("settings.llm.vertexJsonPasteLabel")}
          </label>
          <textarea
            className="vertex-json-input"
            rows={storedValid ? 3 : 6}
            value={repairingInvalid ? apiKey : pasteText}
            autoComplete="off"
            spellCheck={false}
            placeholder={t("settings.llm.vertexJsonPastePlaceholder")}
            onChange={(e) => onPasteChange(e.target.value)}
            onBlur={(e) => onPasteBlur(e.target.value)}
          />
        </>
      )}

      <div className={`hint ${error ? "is-error" : ""}`}>{error || hint}</div>
    </div>
  );
}
