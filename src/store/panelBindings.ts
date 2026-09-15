import { useFileExplorer } from "./fileExplorer";
import { useProject } from "./project";
import { useReader } from "./reader";
import { useReaderFind } from "./readerFind";
import { useRightPanel } from "./rightPanel";
import { useSession } from "./session";

/**
 * Single place where right-panel state is bound to a session.
 *
 * Wired as a module-level store subscription rather than a React effect: the
 * panel (and its file tree) must follow the active session even while the
 * panel is collapsed, the chat view is unmounted (settings / usage routes), or
 * the session changed through a path other than `switchTo`. Anything reading
 * panel state is therefore guaranteed to see only the current session's tabs.
 */
function projectRootOf(sessionId: string | null): string | null {
  if (!sessionId) return null;
  const state = useSession.getState();
  const projectId =
    state.activeId === sessionId
      ? (state.active?.session.project_id ??
        state.sessions.find((s) => s.id === sessionId)?.project_id ??
        null)
      : (state.sessions.find((s) => s.id === sessionId)?.project_id ?? null);
  if (!projectId) return null;
  return useProject.getState().projects.find((p) => p.id === projectId)?.path?.trim() || null;
}

function bind(sessionId: string | null) {
  // Reader docs first: the panel's path ops subscription reads reader state.
  useReader.getState().bindSession(sessionId);
  useRightPanel.getState().bindSession(sessionId);
  useFileExplorer.getState().bindSession(sessionId, projectRootOf(sessionId));
  useReaderFind.getState().reset();
}

let boundSessionId = useSession.getState().activeId;
bind(boundSessionId);

useSession.subscribe((state) => {
  if (state.activeId === boundSessionId) return;
  boundSessionId = state.activeId;
  bind(boundSessionId);
});

// A project path can be attached to an already-open session; keep the file
// tree root in step without rebinding the panel tabs.
let boundProjects = useProject.getState().projects;
useProject.subscribe((state) => {
  if (state.projects === boundProjects) return;
  boundProjects = state.projects;
  if (!boundSessionId) return;
  useFileExplorer.getState().bindSession(boundSessionId, projectRootOf(boundSessionId));
});

export function bindPanelSession(sessionId: string | null) {
  if (sessionId === boundSessionId) return;
  boundSessionId = sessionId;
  bind(sessionId);
}
