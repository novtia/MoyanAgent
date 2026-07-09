import { useState, useRef, useEffect, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { SessionList } from "./SessionList";
import { SessionItem } from "./SessionItem";
import { ProjectConfigModal } from "./ProjectConfigModal";
import { useSession } from "../../store/session";
import { useProject } from "../../store/project";
import { api } from "../../api/tauri";
import type { Project, SessionSummary } from "../../types";
import { sanitizeFsPath } from "../../utils/sanitizePath";
import { openContextMenu } from "../context-menu";
import { toast, dialog } from "../ui";

interface SidebarProps {
  onOpenSettings: () => void;
  onOpenChat: () => void;
  onOpenSearch: () => void;
  settingsActive: boolean;
}

export function Sidebar({
  onOpenSettings,
  onOpenChat,
  onOpenSearch,
  settingsActive,
}: SidebarProps) {
  const { t } = useTranslation();
  const createNew = useSession((s) => s.createNew);
  const sessions = useSession((s) => s.sessions);
  const projects = useProject((s) => s.projects);
  const hasProjects = projects.length > 0;

  const onNewChat = async () => {
    await createNew();
    onOpenChat();
  };

  return (
    <aside className="side">
      <div className="side-top">
        <nav className="side-nav">
          <button type="button" className="side-nav-item" onClick={onNewChat}>
            <NewChatIcon />
            <span>{t("sidebar.newChat")}</span>
          </button>
          <button type="button" className="side-nav-item" onClick={onOpenSearch}>
            <SearchIcon />
            <span>{t("sidebar.search")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <SkillsIcon />
            <span>{t("sidebar.skills")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <PluginIcon />
            <span>{t("sidebar.plugins")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <ClockIcon />
            <span>{t("sidebar.automations")}</span>
          </button>
          {/* 无项目时显示 nav 项；有项目时移到 scroll 区的标题栏 */}
          {!hasProjects && <ProjectNavItem />}
        </nav>

        <div className="side-section">
          <div className="side-section-scroll">
            {/* ── 进行中会话（动态区域，滚动时固定在顶部）── */}
            <ActiveSessionsSection onOpenChat={onOpenChat} />

            {/* ── 项目区块（有项目时显示）── */}
            {hasProjects && (
              <div className="side-project-section">
                <ProjectSectionHeader />
                {projects.map((project) => (
                  <ProjectItem
                    key={project.id}
                    project={project}
                    sessions={sessions.filter((s) => s.project_id === project.id)}
                    onOpenChat={onOpenChat}
                  />
                ))}
              </div>
            )}

            {/* ── 对话区块 ── */}
            <div className="side-section-header">
              <span className="side-section-title-text">{t("sidebar.chats")}</span>
            </div>
            <SessionList onOpenChat={onOpenChat} unassignedOnly={hasProjects} />
            {sessions.filter((s) => !hasProjects || !s.project_id).length === 0 && (
              <div className="side-empty">{t("sidebar.empty")}</div>
            )}
          </div>
        </div>
      </div>

      <div className="side-bottom">
        <button
          type="button"
          className={`side-nav-item ${settingsActive ? "active" : ""}`}
          onClick={onOpenSettings}
        >
          <GearIcon />
          <span>{t("sidebar.settings")}</span>
        </button>
      </div>
    </aside>
  );
}

// ─── Active (in-progress) sessions section ───────────────────────────────────

interface ActiveSessionsSectionProps {
  onOpenChat: () => void;
}

function ActiveSessionsSection({ onOpenChat }: ActiveSessionsSectionProps) {
  const { t } = useTranslation();
  const sessions = useSession((s) => s.sessions);
  const busyBySession = useSession((s) => s.busyBySession);
  const activeId = useSession((s) => s.activeId);
  const switchTo = useSession((s) => s.switchTo);

  const busySessions = sessions.filter((s) => busyBySession[s.id]);
  if (busySessions.length === 0) return null;

  return (
    <div className="side-active-section">
      <div className="side-section-header side-active-section-header">
        <span className="side-active-dot" aria-hidden="true" />
        <span className="side-section-title-text">{t("sidebar.activeChats")}</span>
      </div>
      <div className="chat-list">
        {busySessions.map((s) => (
          <button
            key={s.id}
            type="button"
            className={`chat-item side-active-item ${activeId === s.id ? "active" : ""}`}
            title={s.title}
            onClick={() => {
              switchTo(s.id);
              onOpenChat();
            }}
          >
            <span className="side-active-item-dot" aria-hidden="true" />
            <span className="chat-title">{s.title}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

// ─── Shared project creation logic ───────────────────────────────────────────

function useProjectActions() {
  const projects = useProject((s) => s.projects);
  const createBlank = useProject((s) => s.createBlank);
  const createFromFolder = useProject((s) => s.createFromFolder);
  const reorder = useProject((s) => s.reorder);
  const importArchive = useProject((s) => s.importArchive);
  const [showSortHint, setShowSortHint] = useState(false);
  const [importing, setImporting] = useState(false);

  const handleCreateBlank = async () => {
    const name = await dialog.prompt("请输入项目名称", { placeholder: "新项目" });
    if (!name?.trim()) return;
    await createBlank(name.trim());
  };

  const handleCreateFromFolder = async () => {
    const result = await openDialog({ directory: true, multiple: false, title: "选择项目文件夹" });
    if (!result) return;
    const folderPath = sanitizeFsPath(result as string);
    const parts = folderPath.replace(/\\/g, "/").split("/");
    const folderName = parts[parts.length - 1] || "新项目";
    await createFromFolder(folderName, folderPath);
  };

  const handleImportArchive = async () => {
    const archivePath = await openDialog({
      multiple: false,
      title: "选择 .atelier 归档文件",
      filters: [{ name: "Atelier 归档", extensions: ["atelier"] }],
    });
    if (!archivePath) return;
    setImporting(true);
    try {
      const result = await importArchive(archivePath as string);
      toast.success(
        "导入成功",
        {
          description: `项目 ${result.projects_imported} 个 · 会话 ${result.sessions_imported} 个 · 消息 ${result.messages_imported} 条`,
          duration: 5000,
        },
      );
    } catch (err) {
      toast.error("导入失败", { description: String(err) });
    } finally {
      setImporting(false);
    }
  };

  const handleSort = () => {
    const sorted = [...projects].sort((a, b) => a.name.localeCompare(b.name));
    reorder(sorted.map((p) => p.id));
    setShowSortHint(true);
    setTimeout(() => setShowSortHint(false), 1500);
  };

  return { handleCreateBlank, handleCreateFromFolder, handleImportArchive, handleSort, showSortHint, importing };
}

// ─── Shared action buttons (sort + new project dropdown) ──────────────────────

interface ProjectActionBtnsProps {
  onSort: () => void;
  sortHint: boolean;
}

function ProjectActionBtns({ onSort, sortHint }: ProjectActionBtnsProps) {
  const { handleCreateBlank, handleCreateFromFolder, handleImportArchive, importing } = useProjectActions();
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const newBtnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!dropdownOpen) return;
    const handler = (e: MouseEvent) => {
      if (
        dropdownRef.current && !dropdownRef.current.contains(e.target as Node) &&
        newBtnRef.current && !newBtnRef.current.contains(e.target as Node)
      ) {
        setDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [dropdownOpen]);

  const onCreateBlank = async () => {
    setDropdownOpen(false);
    await handleCreateBlank();
  };

  const onCreateFromFolder = async () => {
    setDropdownOpen(false);
    await handleCreateFromFolder();
  };

  const onImport = async () => {
    setDropdownOpen(false);
    await handleImportArchive();
  };

  return (
    <div className="side-project-action-btns">
      <button
        type="button"
        className="side-icon-btn"
        title={sortHint ? "已排序" : "按名称排序"}
        onClick={(e) => { e.stopPropagation(); onSort(); }}
      >
        <SortIcon />
      </button>
      <div className="side-nav-dropdown-anchor">
        <button
          ref={newBtnRef}
          type="button"
          className="side-icon-btn"
          title="新建 / 导入项目"
          onClick={(e) => { e.stopPropagation(); setDropdownOpen((v) => !v); }}
        >
          <PlusIcon />
        </button>
        {dropdownOpen && (
          <div ref={dropdownRef} className="side-nav-dropdown">
            <button type="button" className="side-nav-dropdown-item" onClick={onCreateBlank}>
              <FolderPlusIcon />
              <span>创建空白项目</span>
            </button>
            <button type="button" className="side-nav-dropdown-item" onClick={onCreateFromFolder}>
              <FolderOpenIcon />
              <span>使用现有文件夹</span>
            </button>
            <div className="side-nav-dropdown-separator" />
            <button
              type="button"
              className="side-nav-dropdown-item"
              onClick={onImport}
              disabled={importing}
            >
              <ImportIcon />
              <span>{importing ? "导入中…" : "从归档导入"}</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Project nav item（无项目时在 nav 显示）────────────────────────────────────

function ProjectNavItem() {
  const { t } = useTranslation();
  const { handleSort, showSortHint } = useProjectActions();

  return (
    <div className="side-nav-project-row">
      <div className="side-nav-item side-nav-item--label">
        <FolderIcon />
        <span>{t("sidebar.project")}</span>
      </div>
      <ProjectActionBtns onSort={handleSort} sortHint={showSortHint} />
    </div>
  );
}

// ─── Project section header（有项目时在 scroll 区显示，替代 nav 项）─────────────

function ProjectSectionHeader() {
  const { handleSort, showSortHint } = useProjectActions();

  return (
    <div className="side-section-header side-section-header--sticky side-section-header--hoverable">
      <span className="side-section-title-text">项目</span>
      <ProjectActionBtns onSort={handleSort} sortHint={showSortHint} />
    </div>
  );
}

// ─── Project item ─────────────────────────────────────────────────────────────

const PROJECT_EXPANDED_KEY = "atelier.sidebar.projectExpanded";

function readProjectExpandedMap(): Record<string, boolean> {
  try {
    const raw = window.localStorage.getItem(PROJECT_EXPANDED_KEY);
    return raw ? (JSON.parse(raw) as Record<string, boolean>) : {};
  } catch {
    return {};
  }
}

function writeProjectExpanded(projectId: string, expanded: boolean) {
  const next = { ...readProjectExpandedMap(), [projectId]: expanded };
  window.localStorage.setItem(PROJECT_EXPANDED_KEY, JSON.stringify(next));
}

interface ProjectItemProps {
  project: Project;
  sessions: SessionSummary[];
  onOpenChat: () => void;
}

function ProjectItem({ project, sessions, onOpenChat }: ProjectItemProps) {
  const { t } = useTranslation();
  const rename = useProject((s) => s.rename);
  const remove = useProject((s) => s.remove);
  const exportProjects = useProject((s) => s.exportProjects);
  const createNew = useSession((s) => s.createNew);
  const refreshSessionList = useSession((s) => s.refreshList);
  const reloadActiveSession = useSession((s) => s.reloadActiveSession);
  const activeId = useSession((s) => s.activeId);
  const [configOpen, setConfigOpen] = useState(false);
  const [expanded, setExpanded] = useState(
    () => readProjectExpandedMap()[project.id] ?? true,
  );

  useEffect(() => {
    if (!sessions.some((s) => s.id === activeId)) return;
    setExpanded(true);
    writeProjectExpanded(project.id, true);
  }, [activeId, project.id, sessions]);

  const toggleExpanded = () => {
    setExpanded((current) => {
      const next = !current;
      writeProjectExpanded(project.id, next);
      return next;
    });
  };

  const handleNewSession = async (e: ReactMouseEvent) => {
    e.stopPropagation();
    const sessionId = await createNew();
    await api.assignSessionToProject(sessionId, project.id);
    // Re-hydrate the active session so its project_id (and thus the shared
    // project-level agent flow) is reflected immediately.
    await reloadActiveSession();
    await refreshSessionList();
    setExpanded(true);
    writeProjectExpanded(project.id, true);
    onOpenChat();
  };

  const handleExportProject = async () => {
    const destPath = await saveDialog({
      title: "导出项目归档",
      defaultPath: `${project.name}.atelier`,
      filters: [{ name: "Atelier 归档", extensions: ["atelier"] }],
    });
    if (!destPath) return;
    try {
      await exportProjects([project.id], destPath as string);
      toast.success("导出成功", { description: destPath as string });
    } catch (err) {
      toast.error("导出失败", { description: String(err) });
    }
  };

  const openProjectMenu = (e: ReactMouseEvent) => {
    e.stopPropagation();
    openContextMenu(e, [
      {
        id: "project-settings",
        label: "项目设置",
        onSelect: () => setConfigOpen(true),
      },
      {
        id: "project-rename",
        label: "重命名项目",
        onSelect: async () => {
          const name = await dialog.prompt("请输入新项目名称", { defaultValue: project.name });
          if (name?.trim()) rename(project.id, name.trim());
        },
      },
      {
        id: "project-export",
        label: "导出项目",
        onSelect: handleExportProject,
      },
      { type: "separator" },
      {
        id: "project-delete",
        label: "删除项目",
        danger: true,
        onSelect: async () => {
          const ok = await dialog.confirm(
            `删除项目「${project.name}」？\n项目下的会话不会被删除。`,
            { type: "danger", confirmLabel: "删除", title: "删除项目" },
          );
          if (ok) remove(project.id);
        },
      },
    ]);
  };

  return (
    <>
    <div className="project-item">
      <div
        className="project-item-header"
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        aria-label={expanded ? t("sidebar.collapseProject") : t("sidebar.expandProject")}
        title={expanded ? t("sidebar.collapseProject") : t("sidebar.expandProject")}
        onClick={toggleExpanded}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleExpanded();
          }
        }}
        onContextMenu={openProjectMenu}
      >
        <span className="project-item-folder-icon">
          <FolderIcon />
        </span>
        <span className="project-item-name" title={project.name}>
          {project.name}
        </span>
        <div className="project-item-trailing">
          {!expanded && sessions.length > 0 && (
            <span className="project-item-count">{sessions.length}</span>
          )}
          <div className="project-item-actions">
            <button
              type="button"
              className="side-icon-btn"
              title="在此项目中新建会话"
              onClick={handleNewSession}
            >
              <NewChatIcon />
            </button>
            <button
              type="button"
              className="side-icon-btn"
              title="项目选项"
              onClick={openProjectMenu}
            >
              <DotsIcon />
            </button>
          </div>
        </div>
      </div>

      <div
        className={`project-sessions-wrap${expanded ? " is-expanded" : ""}`}
        aria-hidden={!expanded}
      >
        <div className="project-sessions-inner">
          <div className="project-sessions">
            {sessions.length === 0 ? (
              <div className="project-sessions-empty">{t("sidebar.noProjectSessions")}</div>
            ) : (
              sessions.map((s) => (
                <SessionItem
                  key={s.id}
                  session={s}
                  isActive={activeId === s.id}
                  className="chat-item--nested"
                  onOpenChat={onOpenChat}
                  projectId={project.id}
                  onOpenProjectConfig={() => setConfigOpen(true)}
                />
              ))
            )}
          </div>
        </div>
      </div>
    </div>
    {configOpen && (
      <ProjectConfigModal project={project} onClose={() => setConfigOpen(false)} />
    )}
    </>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────────

function NewChatIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}
function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  );
}
function SkillsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 3 7l9 5 9-5-9-5Z" />
      <path d="m3 12 9 5 9-5" />
      <path d="m3 17 9 5 9-5" />
    </svg>
  );
}
function PluginIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  );
}
function ClockIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}
function FolderIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
    </svg>
  );
}
function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}
function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}
function SortIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 6h18M7 12h10M11 18h2" />
    </svg>
  );
}
function FolderPlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
      <path d="M12 11v6M9 14h6" />
    </svg>
  );
}
function FolderOpenIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M5 19a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1" />
      <path d="M21 19H7l-2-7h18l-2 7z" />
    </svg>
  );
}
function DotsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="5" r="1" fill="currentColor" />
      <circle cx="12" cy="12" r="1" fill="currentColor" />
      <circle cx="12" cy="19" r="1" fill="currentColor" />
    </svg>
  );
}
function ImportIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <polyline points="17 8 12 3 7 8" />
      <line x1="12" y1="3" x2="12" y2="15" />
    </svg>
  );
}
