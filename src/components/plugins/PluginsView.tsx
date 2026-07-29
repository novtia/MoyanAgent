import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api/tauri";
import { useSettings } from "../../store/settings";
import type { SkillInfo } from "../../types";
import { toast } from "../ui";
import { ReaderMarkdownPreview } from "../chat/rightPanel/reader/ReaderMarkdownPreview";

type MarketTab = "plugins" | "skills";

function skillGlyph(skill: SkillInfo): string {
  const s = (skill.name || skill.id).trim();
  if (!s) return "?";
  const cjk = s.match(/[\u4e00-\u9fff]/);
  if (cjk) return cjk[0];
  const letter = s.match(/[A-Za-z0-9]/);
  if (letter) return letter[0].toUpperCase();
  return s[0];
}

export function PluginsView() {
  const { t } = useTranslation();
  const loadSettings = useSettings((s) => s.load);
  const [tab, setTab] = useState<MarketTab>("skills");
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const list = await api.listSkills();
      setSkills(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selected = useMemo(
    () => skills.find((s) => s.id === selectedId) ?? null,
    [skills, selectedId],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return skills;
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.id.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((tag) => tag.toLowerCase().includes(q)),
    );
  }, [skills, query]);

  const enabled = filtered.filter((s) => s.enabled);
  const disabled = filtered.filter((s) => !s.enabled);

  const toggleEnabled = async (skill: SkillInfo, next: boolean) => {
    try {
      await api.setSkillEnabled(skill.id, next);
      await loadSettings();
      setSkills((prev) =>
        prev.map((s) => (s.id === skill.id ? { ...s, enabled: next } : s)),
      );
    } catch (e) {
      toast.error(String(e));
    }
  };

  const onImport = async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "SKILL.md", extensions: ["md"] }],
      });
      if (!picked || typeof picked !== "string") return;
      setBusy(true);
      const info = await api.importSkill(picked);
      await refresh();
      setSelectedId(info.id);
      toast.success(t("plugins.imported", { name: info.name }));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onUninstall = async (skill: SkillInfo) => {
    if (skill.source === "builtin") {
      toast.error(t("plugins.cannotUninstallBuiltin"));
      return;
    }
    try {
      await api.uninstallSkill(skill.id);
      if (selectedId === skill.id) setSelectedId(null);
      await refresh();
      toast.success(t("plugins.uninstalled", { name: skill.name }));
    } catch (e) {
      toast.error(String(e));
    }
  };

  if (selected) {
    return (
      <div className={`plugins ${busy ? "is-refreshing" : ""}`}>
        <div className="plugins-panel">
          <button
            type="button"
            className="plugins-back"
            onClick={() => setSelectedId(null)}
          >
            ← {t("plugins.backToList")}
          </button>
          <SkillDetail
            skill={selected}
            onToggle={(next) => void toggleEnabled(selected, next)}
            onUninstall={() => void onUninstall(selected)}
          />
        </div>
      </div>
    );
  }

  return (
    <div className={`plugins ${busy ? "is-refreshing" : ""}`}>
      <div className="plugins-panel">
        <header className="plugins-header">
          <div className="plugins-header-main">
            <h1 className="plugins-header-title">{t("plugins.title")}</h1>
            <p className="plugins-header-subtitle">{t("plugins.subtitle")}</p>
          </div>
          <div className="plugins-header-actions">
            <div className="plugins-seg" role="tablist">
              <button
                type="button"
                className={tab === "plugins" ? "on" : ""}
                onClick={() => setTab("plugins")}
              >
                {t("plugins.tabPlugins")}
              </button>
              <button
                type="button"
                className={tab === "skills" ? "on" : ""}
                onClick={() => setTab("skills")}
              >
                {t("plugins.tabSkills")}
              </button>
            </div>
            {tab === "skills" && (
              <>
                <label className="plugins-search">
                  <span className="plugins-search-icon" aria-hidden>
                    <svg
                      width="13"
                      height="13"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <circle cx="11" cy="11" r="8" />
                      <line x1="21" y1="21" x2="16.65" y2="16.65" />
                    </svg>
                  </span>
                  <input
                    type="search"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder={t("plugins.searchSkills")}
                  />
                </label>
                <button
                  type="button"
                  className="plugins-btn-icon"
                  onClick={() => void refresh()}
                  title={t("plugins.refresh")}
                  aria-label={t("plugins.refresh")}
                >
                  <svg
                    width="13"
                    height="13"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                    <path d="M3 3v5h5" />
                    <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
                    <path d="M16 16h5v5" />
                  </svg>
                </button>
                <button type="button" className="plugins-btn-primary" onClick={() => void onImport()}>
                  {t("plugins.importSkill")}
                </button>
              </>
            )}
          </div>
        </header>

        {error && <div className="plugins-error">{error}</div>}

        {tab === "plugins" ? (
          <div className="plugins-empty">
            <div className="plugins-empty-title">{t("plugins.pluginsComingTitle")}</div>
            <div className="plugins-empty-desc">{t("plugins.pluginsComingDesc")}</div>
          </div>
        ) : (
          <>
            {enabled.length > 0 && (
              <section className="plugins-section">
                <div className="plugins-section-title">
                  {t("plugins.sectionEnabled")}
                  <span className="plugins-section-count">{enabled.length}</span>
                </div>
                <div className="plugins-skill-grid">
                  {enabled.map((skill) => (
                    <SkillCard
                      key={skill.id}
                      skill={skill}
                      onOpen={() => setSelectedId(skill.id)}
                      onToggle={(next) => void toggleEnabled(skill, next)}
                    />
                  ))}
                </div>
              </section>
            )}
            {disabled.length > 0 && (
              <section className="plugins-section">
                <div className="plugins-section-title">
                  {t("plugins.sectionAvailable")}
                  <span className="plugins-section-count">{disabled.length}</span>
                </div>
                <div className="plugins-skill-grid">
                  {disabled.map((skill) => (
                    <SkillCard
                      key={skill.id}
                      skill={skill}
                      onOpen={() => setSelectedId(skill.id)}
                      onToggle={(next) => void toggleEnabled(skill, next)}
                    />
                  ))}
                </div>
              </section>
            )}
            {filtered.length === 0 && (
              <div className="plugins-empty">
                <div className="plugins-empty-title">{t("plugins.noSkills")}</div>
                <div className="plugins-empty-desc">{t("plugins.noSkillsDesc")}</div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function SkillCard({
  skill,
  onOpen,
  onToggle,
}: {
  skill: SkillInfo;
  onOpen: () => void;
  onToggle: (next: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <button type="button" className="plugins-skill-card" onClick={onOpen}>
      <span className="plugins-skill-icon">{skillGlyph(skill)}</span>
      <span className="plugins-skill-main">
        <span className="plugins-skill-top">
          <span className="plugins-skill-name">{skill.name}</span>
          {skill.version && (
            <span className="plugins-skill-ver">v{skill.version}</span>
          )}
          {skill.source === "builtin" && (
            <span className="plugins-chip sys">{t("plugins.chipSystem")}</span>
          )}
          <span className="plugins-spacer" />
          <span
            className={`plugins-switch ${skill.enabled ? "on" : ""}`}
            role="switch"
            aria-checked={skill.enabled}
            onClick={(e) => {
              e.stopPropagation();
              onToggle(!skill.enabled);
            }}
          />
        </span>
        <span className="plugins-skill-desc">{skill.description}</span>
        <span className="plugins-skill-foot">
          <span className="plugins-skill-author">
            {skill.author || (skill.source === "builtin" ? "Lumen" : t("plugins.localImport"))}
          </span>
          {skill.enabled && (
            <span className="plugins-chip on">{t("plugins.chipEnabled")}</span>
          )}
        </span>
      </span>
    </button>
  );
}

function SkillDetail({
  skill,
  onToggle,
  onUninstall,
}: {
  skill: SkillInfo;
  onToggle: (next: boolean) => void;
  onUninstall: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="plugins-detail">
      <div className="plugins-detail-head">
        <span className="plugins-detail-icon">{skillGlyph(skill)}</span>
        <div className="plugins-detail-title">
          <div className="plugins-detail-name">{skill.name}</div>
          <div className="plugins-detail-tagline">{skill.description}</div>
          <div className="plugins-detail-meta">
            <span>{skill.author || "Lumen"}</span>
            <span className="sep">·</span>
            {skill.version && (
              <>
                <span className="mono">v{skill.version}</span>
                <span className="sep">·</span>
              </>
            )}
            {skill.source === "builtin" ? (
              <span className="plugins-chip sys">{t("plugins.chipSystem")}</span>
            ) : (
              <span className="plugins-chip">{t("plugins.localImport")}</span>
            )}
          </div>
        </div>
        <div className="plugins-detail-actions">
          <button
            type="button"
            className={skill.enabled ? "plugins-btn-ghost" : "plugins-btn-primary"}
            onClick={() => onToggle(!skill.enabled)}
          >
            {skill.enabled ? t("plugins.disable") : t("plugins.enable")}
          </button>
          {skill.source !== "builtin" && (
            <button type="button" className="plugins-btn-ghost" onClick={onUninstall}>
              {t("plugins.uninstall")}
            </button>
          )}
        </div>
      </div>

      <div className="plugins-detail-tabs" role="tablist">
        <button type="button" className="plugins-detail-tab on">
          {t("plugins.tabDetails")}
        </button>
      </div>

      <div className="plugins-detail-body">
        <div className="plugins-detail-readme">
          <ReaderMarkdownPreview
            text={skill.body || skill.description}
            className="plugins-detail-md"
          />
        </div>
        <aside className="plugins-detail-side">
          <div className="plugins-side-block">
            <h4>{t("plugins.installInfo")}</h4>
            <div className="plugins-side-rows">
              <div className="plugins-side-row">
                <span className="k">{t("plugins.metaId")}</span>
                <span className="v mono">{skill.id}</span>
              </div>
              <div className="plugins-side-row">
                <span className="k">{t("plugins.metaVersion")}</span>
                <span className="v mono">{skill.version || "—"}</span>
              </div>
              <div className="plugins-side-row">
                <span className="k">{t("plugins.metaSource")}</span>
                <span className="v">
                  {skill.source === "builtin"
                    ? t("plugins.sourceBuiltin")
                    : t("plugins.sourceUser")}
                </span>
              </div>
            </div>
          </div>
          {skill.tags.length > 0 && (
            <div className="plugins-side-block">
              <h4>{t("plugins.categories")}</h4>
              <div className="plugins-tag-row">
                {skill.tags.map((tag) => (
                  <span key={tag} className="plugins-chip">
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          )}
          <div className="plugins-side-block">
            <h4>{t("plugins.resources")}</h4>
            <div className="plugins-side-rows">
              <div className="plugins-side-row">
                <span className="k">{t("plugins.metaFile")}</span>
                <span className="v mono">SKILL.md</span>
              </div>
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}
