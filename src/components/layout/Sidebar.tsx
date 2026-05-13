import { useState, useRef, useEffect, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { SessionList } from "./SessionList";
import { SessionItem } from "./SessionItem";
import { ProjectConfigModal } from "./ProjectConfigModal";
import { useSession } from "../../store/session";
import { useProject } from "../../store/project";
import { api } from "../../api/tauri";
import type { Project, SessionSummary } from "../../types";
import { openContextMenu } from "../context-menu";

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

// ─── Shared project creation logic ───────────────────────────────────────────

function useProjectActions() {
  const projects = useProject((s) => s.projects);
  const createBlank = useProject((s) => s.createBlank);
  const createFromFolder = useProject((s) => s.createFromFolder);
  const reorder = useProject((s) => s.reorder);
  const [showSortHint, setShowSortHint] = useState(false);

  const handleCreateBlank = async () => {
    const name = prompt("项目名称");
    if (!name?.trim()) return;
    await createBlank(name.trim());
  };

  const handleCreateFromFolder = async () => {
    const result = await openDialog({ directory: true, multiple: false, title: "选择项目文件夹" });
    if (!result) return;
    const folderPath = result as string;
    const parts = folderPath.replace(/\\/g, "/").split("/");
    const folderName = parts[parts.length - 1] || "新项目";
    await createFromFolder(folderName, folderPath);
  };

  const handleSort = () => {
    const sorted = [...projects].sort((a, b) => a.name.localeCompare(b.name));
    reorder(sorted.map((p) => p.id));
    setShowSortHint(true);
    setTimeout(() => setShowSortHint(false), 1500);
  };

  return { handleCreateBlank, handleCreateFromFolder, handleSort, showSortHint };
}

// ─── Shared action buttons (sort + new project dropdown) ──────────────────────

interface ProjectActionBtnsProps {
  onSort: () => void;
  sortHint: boolean;
}

function ProjectActionBtns({ onSort, sortHint }: ProjectActionBtnsProps) {
  const { handleCreateBlank, handleCreateFromFolder } = useProjectActions();
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
          title="新建项目"
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

interface ProjectItemProps {
  project: Project;
  sessions: SessionSummary[];
  onOpenChat: () => void;
}

function ProjectItem({ project, sessions, onOpenChat }: ProjectItemProps) {
  const rename = useProject((s) => s.rename);
  const remove = useProject((s) => s.remove);
  const createNew = useSession((s) => s.createNew);
  const refreshSessionList = useSession((s) => s.refreshList);
  const activeId = useSession((s) => s.activeId);
  const [configOpen, setConfigOpen] = useState(false);

  const handleNewSession = async (e: ReactMouseEvent) => {
    e.stopPropagation();
    const sessionId = await createNew();
    await api.assignSessionToProject(sessionId, project.id);
    await refreshSessionList();
    onOpenChat();
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
        onSelect: () => {
          const name = prompt("新项目名称", project.name);
          if (name?.trim()) rename(project.id, name.trim());
        },
      },
      { type: "separator" },
      {
        id: "project-delete",
        label: "删除项目",
        danger: true,
        onSelect: () => {
          if (window.confirm(`删除项目「${project.name}」？项目下的会话不会被删除。`)) {
            remove(project.id);
          }
        },
      },
    ]);
  };

  return (
    <>
    <div className="project-item">
      <div className="project-item-header" onContextMenu={openProjectMenu}>
        {/* 文件夹图标 */}
        <span className="project-item-folder-icon">
          <FolderIcon />
        </span>
        {/* 项目名 */}
        <span className="project-item-name" title={project.name}>
          {project.name}
        </span>
        {/* 悬停显示的操作按钮 */}
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

      <div className="project-sessions">
        {sessions.length === 0 ? (
          <div className="project-sessions-empty">暂无会话</div>
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
