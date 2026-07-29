import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Role } from "../store/roleState";
import type {
  AgentDefinitionInfo,
  AgentSummary,
  AttachmentDraft,
  BackupListItem,
  BackupModule,
  BackupResult,
  BackupStatus,
  ChainEntry,
  CustomAgent,
  EditOp,
  GenerateResult,
  ImageRefAbs,
  ImportResult,
  LlmModelCatalog,
  MessageAbs,
  ModelParamSettings,
  Project,
  ProjectDirEntry,
  ProjectRule,
  ProjectTextFile,
  PendingDiffRow,
  PendingDiffRevert,
  RemoteModelInfo,
  RestoreResult,
  SessionSearchResult,
  SessionSearchHit,
  SessionSummary,
  SessionWithMessagesAbs,
  MessageOutlineItem,
  DailyUsageRow,
  TokenUsageEventRow,
  TokenUsageSummary,
  ToolUsageRow,
  Session,
  Settings,
  SettingsPatch,
  SkillInfo,
  WebSearchOutcome,
} from "../types";

/** Per-session fields the backend merges into generation (debug log only). */
function sessionSettingsForLog(s: Session) {
  return {
    session_id: s.id,
    title: s.title,
    model: s.model,
    system_prompt: s.system_prompt,
    history_turns: s.history_turns,
    llm_params: s.llm_params,
    agent_type: s.agent_type,
  };
}

export const api = {
  // settings
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<Settings>("update_settings", { patch }),
  getLlmModelCatalog: () => invoke<LlmModelCatalog>("get_llm_model_catalog"),
  fetchProviderModels: (sdk: string, endpoint: string, apiKey: string) =>
    invoke<RemoteModelInfo[]>("fetch_provider_models", {
      args: { sdk, endpoint, apiKey },
    }),
  webSearch: (query: string, maxResults?: number) =>
    invoke<WebSearchOutcome>("web_search", {
      query,
      maxResults: maxResults ?? null,
    }),

  // app info
  getAppInfo: () =>
    invoke<{
      version: string;
      data_dir: string;
      db_path: string;
      sessions_dir: string;
    }>("get_app_info"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  toggleDevtools: () => invoke<void>("toggle_devtools"),

  // sessions
  listSessions: () => invoke<SessionSummary[]>("list_sessions"),
  searchSessions: (query: string, limit = 20) =>
    invoke<SessionSearchResult[]>("search_sessions", { query, limit }),
  searchSessionHits: (sessionId: string, query: string, limit = 200) =>
    invoke<SessionSearchHit[]>("search_session_hits", {
      sessionId,
      query,
      limit,
    }),
  createSession: (title?: string, model?: string) =>
    invoke<Session>("create_session", { args: { title, model } }),
  renameSession: (id: string, title: string) =>
    invoke<void>("rename_session", { id, title }),
  updateSessionConfig: (
    id: string,
    systemPrompt: string,
    historyTurns: number,
    llmParams: ModelParamSettings,
  ) =>
    invoke<void>("update_session_config", {
      args: { id, systemPrompt, historyTurns, llmParams },
    }),
  setSessionModel: (
    id: string,
    model: string,
    contextWindow: number | null,
    providerId?: string | null,
  ) =>
    invoke<void>("set_session_model", {
      args: { id, model, contextWindow, providerId: providerId ?? null },
    }),
  setSessionAgentType: (id: string, agentType: string) =>
    invoke<void>("set_session_agent_type", {
      args: { id, agentType },
    }),
  setSessionAgentChain: (id: string, chain: ChainEntry[]) =>
    invoke<void>("set_session_agent_chain", {
      args: { id, chain },
    }),
  setProjectAgentChain: (id: string, chain: ChainEntry[]) =>
    invoke<void>("set_project_agent_chain", {
      args: { id, chain },
    }),
  deleteSession: (id: string) => invoke<void>("delete_session", { id }),
  loadSession: (id: string) =>
    invoke<SessionWithMessagesAbs>("load_session", { id }),
  listMessageOutline: (sessionId: string) =>
    invoke<MessageOutlineItem[]>("list_message_outline", { sessionId }),
  listMessagesWindow: (args: {
    sessionId: string;
    aroundMessageId?: string | null;
    beforeCreatedAt?: number | null;
    afterCreatedAt?: number | null;
    limit?: number;
  }) =>
    invoke<MessageAbs[]>("list_messages_window", {
      args: {
        sessionId: args.sessionId,
        aroundMessageId: args.aroundMessageId ?? null,
        beforeCreatedAt: args.beforeCreatedAt ?? null,
        afterCreatedAt: args.afterCreatedAt ?? null,
        limit: args.limit ?? null,
      },
    }),
  loadSessionWindow: (
    id: string,
    aroundMessageId?: string | null,
    limit = 60,
  ) =>
    invoke<SessionWithMessagesAbs>("load_session_window", {
      id,
      aroundMessageId: aroundMessageId ?? null,
      limit,
    }),
  listSessionMedia: (sessionId: string) =>
    invoke<ImageRefAbs[]>("list_session_media", { sessionId }),
  assignSessionToProject: (sessionId: string, projectId: string | null) =>
    invoke<void>("assign_session_to_project", { sessionId, projectId }),

  // projects
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string, path?: string | null) =>
    invoke<Project>("create_project", { args: { name, path: path ?? null } }),
  renameProject: (id: string, name: string) =>
    invoke<void>("rename_project", { id, name }),
  updateProjectPath: (id: string, path: string | null) =>
    invoke<void>("update_project_path", { id, path }),
  deleteProject: (id: string) => invoke<void>("delete_project", { id }),
  reorderProjects: (orderedIds: string[]) =>
    invoke<void>("reorder_projects", { orderedIds }),
  updateProjectConfig: (
    id: string,
    systemPrompt: string,
    historyTurns: number,
    llmParams: ModelParamSettings,
    contextWindow: number | null,
  ) =>
    invoke<void>("update_project_config", {
      args: { id, systemPrompt, historyTurns, llmParams, contextWindow },
    }),
  deleteMessage: (id: string) => invoke<void>("delete_message", { id }),
  updateMessageText: (id: string, text: string) =>
    invoke<void>("update_message_text", { id, text }),
  updateMessageImages: (id: string, imageIds: string[]) =>
    invoke<MessageAbs>("update_message_images", { id, imageIds }),
  quoteMessageAsAttachments: (sessionId: string, messageId: string) =>
    invoke<AttachmentDraft[]>("quote_message_as_attachments", {
      sessionId,
      messageId,
    }),
  addAttachmentFromPath: (sessionId: string, path: string) =>
    invoke<AttachmentDraft>("add_attachment_from_path", {
      sessionId,
      path,
    }),
  addAttachmentFromBytes: (sessionId: string, name: string, bytes: Uint8Array) =>
    invoke<AttachmentDraft>("add_attachment_from_bytes", {
      args: { session_id: sessionId, name, bytes },
    }),
  addUrlAttachment: (sessionId: string, url: string) =>
    invoke<AttachmentDraft>("add_url_attachment", {
      args: { session_id: sessionId, url },
    }),
  removeAttachmentDraft: (imageId: string) =>
    invoke<void>("remove_attachment_draft", { imageId }),

  getImageAbsPath: (imageId: string) =>
    invoke<string>("get_image_abs_path", { imageId }),

  // generate
  generateImage: async (
    req: {
      session_id: string;
      prompt: string;
      attachment_ids: string[];
      aspect_ratio: string;
      image_size: string;
      video_mode?: "text" | "first_frame" | "first_last" | "reference";
      video_duration?: number;
      video_resolution?: string;
      generate_audio?: boolean;
      watermark?: boolean;
      camera_fixed?: boolean | null;
      seed?: number | null;
    },
    session?: Session | null,
  ) => {
    const tag = "[atelier] generate_image";
    console.log(`${tag} request →`, {
      ...req,
      session_settings: session ? sessionSettingsForLog(session) : null,
    });
    const res = await invoke<GenerateResult>("generate_image", { req });
    console.log(`${tag} response ←`, res);
    return res;
  },
  regenerateImage: async (
    req: {
      session_id: string;
      user_message_id: string;
      aspect_ratio: string;
      image_size: string;
      video_mode?: "text" | "first_frame" | "first_last" | "reference";
      video_duration?: number;
      video_resolution?: string;
      generate_audio?: boolean;
      watermark?: boolean;
      camera_fixed?: boolean | null;
      seed?: number | null;
    },
    session?: Session | null,
  ) => {
    const tag = "[atelier] regenerate_image";
    console.log(`${tag} request →`, {
      ...req,
      session_settings: session ? sessionSettingsForLog(session) : null,
    });
    const res = await invoke<GenerateResult>("regenerate_image", { req });
    console.log(`${tag} response ←`, res);
    return res;
  },
  cancelGeneration: (sessionId: string) =>
    invoke<void>("cancel_generation", { sessionId }),
  /** Wake a blocked AskUser tool; `promptId` is the tool_use call id. */
  answerAskUser: (
    promptId: string,
    answer: string,
    items: Array<{ prompt: string; answer: string }> = [],
  ) =>
    invoke<boolean>("answer_ask_user", {
      args: { promptId, answer, items },
    }),
  saveCancelledMessage: (
    sessionId: string,
    text: string,
    thinking = "",
    blocks: unknown[] | null = null,
  ) =>
    invoke<void>("save_cancelled_message", {
      sessionId,
      text,
      thinking,
      blocks,
    }),

  // local editing
  editImage: (imageId: string, op: EditOp) =>
    invoke<ImageRefAbs>("edit_image", { args: { image_id: imageId, op } }),

  // export image
  exportImage: (imageId: string, destPath: string) =>
    invoke<void>("export_image", { args: { image_id: imageId, dest_path: destPath } }),
  exportMedia: (mediaId: string, destPath: string) =>
    invoke<void>("export_media", { args: { image_id: mediaId, dest_path: destPath } }),
  exportMediaZip: (imageIds: string[], destPath: string) =>
    invoke<void>("export_media_zip", {
      args: { image_ids: imageIds, dest_path: destPath },
    }),
  deleteMedia: (imageIds: string[]) =>
    invoke<void>("delete_media", { args: { image_ids: imageIds } }),

  // project / session transfer
  exportProjectsArchive: (projectIds: string[], destPath: string) =>
    invoke<void>("export_projects_archive", { projectIds, destPath }),

  exportSessionArchive: (sessionId: string, destPath: string) =>
    invoke<void>("export_session_archive", { sessionId, destPath }),

  importArchive: (archivePath: string) =>
    invoke<ImportResult>("import_archive", { archivePath }),

  createBackup: (module: BackupModule, destPath?: string | null) =>
    invoke<BackupResult>("create_backup", {
      args: { module, destPath: destPath ?? null, kind: "manual" },
    }),
  restoreBackup: (archivePath: string) =>
    invoke<RestoreResult>("restore_backup", {
      args: { archivePath },
    }),
  listBackups: (module?: BackupModule | null) =>
    invoke<BackupListItem[]>("list_backups", {
      args: { module: module ?? null },
    }),
  getBackupStatus: () => invoke<BackupStatus>("get_backup_status"),

  // agents
  listAgents: () => invoke<AgentSummary[]>("list_agents"),
  getAgentDefinition: (agentType: string) =>
    invoke<AgentDefinitionInfo>("get_agent_definition", { agentType }),
  listAgentTools: () => invoke<string[]>("list_agent_tools"),
  listCustomAgents: () => invoke<CustomAgent[]>("list_custom_agents"),
  createCustomAgent: (args: {
    name: string;
    whenToUse?: string;
    systemPrompt?: string;
    model?: string | null;
    tools?: string[];
  }) =>
    invoke<CustomAgent>("create_custom_agent", {
      args: {
        name: args.name,
        whenToUse: args.whenToUse ?? "",
        systemPrompt: args.systemPrompt ?? "",
        model: args.model ?? null,
        tools: args.tools ?? [],
      },
    }),
  updateCustomAgent: (args: {
    agentType: string;
    name: string;
    whenToUse?: string;
    systemPrompt?: string;
    model?: string | null;
    tools?: string[];
  }) =>
    invoke<CustomAgent>("update_custom_agent", {
      args: {
        agentType: args.agentType,
        name: args.name,
        whenToUse: args.whenToUse ?? "",
        systemPrompt: args.systemPrompt ?? "",
        model: args.model ?? null,
        tools: args.tools ?? [],
      },
    }),
  deleteCustomAgent: (agentType: string) =>
    invoke<void>("delete_custom_agent", { args: { agentType } }),

  // role state board
  getRoleStates: (sessionId: string) =>
    invoke<Role[]>("get_role_states", { sessionId }),
  updateRoleState: (sessionId: string, role: Role) =>
    invoke<Role>("update_role_state", { sessionId, role }),
  reorderRoleStates: (sessionId: string, orderedIds: string[]) =>
    invoke<Role[]>("reorder_role_states", { sessionId, orderedIds }),
  deleteRoleState: (sessionId: string, id: string) =>
    invoke<{ removed: boolean }>("delete_role_state", { sessionId, id }),

  writeProjectFile: (
    sessionId: string,
    path: string,
    content: string,
    encoding?: string | null,
    hadBom?: boolean | null,
  ) =>
    invoke<void>("write_project_file", {
      sessionId,
      path,
      content,
      encoding: encoding ?? null,
      hadBom: hadBom ?? null,
    }),

  readProjectFile: (sessionId: string, path: string) =>
    invoke<ProjectTextFile>("read_project_file", { sessionId, path }),

  listPendingDiffs: (sessionId: string, path?: string | null) =>
    invoke<PendingDiffRow[]>("list_pending_diffs", {
      sessionId,
      path: path ?? null,
    }),

  confirmPendingDiff: (sessionId: string, id: string, accept: boolean) =>
    invoke<PendingDiffRevert | null>("confirm_pending_diff", {
      sessionId,
      id,
      accept,
    }),

  confirmAllPendingDiffs: (sessionId: string, path: string, accept: boolean) =>
    invoke<PendingDiffRevert | null>("confirm_all_pending_diffs", {
      sessionId,
      path,
      accept,
    }),

  listProjectDir: (sessionId: string, path?: string | null) =>
    invoke<ProjectDirEntry[]>("list_project_dir", {
      sessionId,
      path: path ?? null,
    }),

  createProjectDir: (sessionId: string, path: string) =>
    invoke<void>("create_project_dir", { sessionId, path }),

  createProjectFile: (sessionId: string, path: string, content?: string | null) =>
    invoke<void>("create_project_file", {
      sessionId,
      path,
      content: content ?? null,
    }),

  renameProjectPath: (sessionId: string, from: string, to: string) =>
    invoke<void>("rename_project_path", { sessionId, from, to }),

  copyProjectPath: (sessionId: string, from: string, to: string) =>
    invoke<void>("copy_project_path", { sessionId, from, to }),

  /** Copy an OS file/folder into the project at an exact destination path. */
  importExternalPathToProject: (
    sessionId: string,
    srcPath: string,
    destPath: string,
  ) =>
    invoke<void>("import_external_path_to_project", {
      sessionId,
      srcPath,
      destPath,
    }),

  /** Write raw bytes to a new project file (drop fallback without native path). */
  writeProjectFileBytes: (
    sessionId: string,
    path: string,
    bytes: Uint8Array,
  ) =>
    invoke<void>("write_project_file_bytes", {
      sessionId,
      path,
      bytes,
    }),

  deleteProjectPath: (sessionId: string, path: string) =>
    invoke<void>("delete_project_path", { sessionId, path }),

  listProjectRules: (sessionId: string) =>
    invoke<ProjectRule[]>("list_project_rules", { sessionId }),

  setProjectRuleEnabled: (sessionId: string, path: string, enabled: boolean) =>
    invoke<void>("set_project_rule_enabled", { sessionId, path, enabled }),

  getTokenUsageSummary: (args?: {
    fromMs?: number | null;
    toMs?: number | null;
  }) =>
    invoke<TokenUsageSummary>("get_token_usage_summary", {
      args: {
        from_ms: args?.fromMs ?? null,
        to_ms: args?.toMs ?? null,
      },
    }),

  getTokenUsageDaily: (args?: {
    fromMs?: number | null;
    toMs?: number | null;
  }) =>
    invoke<DailyUsageRow[]>("get_token_usage_daily", {
      args: {
        from_ms: args?.fromMs ?? null,
        to_ms: args?.toMs ?? null,
      },
    }),

  getTokenUsageByTool: (args?: {
    fromMs?: number | null;
    toMs?: number | null;
  }) =>
    invoke<ToolUsageRow[]>("get_token_usage_by_tool", {
      args: {
        from_ms: args?.fromMs ?? null,
        to_ms: args?.toMs ?? null,
      },
    }),

  listTokenUsageEvents: (args?: {
    sessionId?: string | null;
    model?: string | null;
    eventKind?: string | null;
    fromMs?: number | null;
    toMs?: number | null;
    limit?: number | null;
    offset?: number | null;
  }) =>
    invoke<TokenUsageEventRow[]>("list_token_usage_events", {
      args: {
        session_id: args?.sessionId ?? null,
        model: args?.model ?? null,
        event_kind: args?.eventKind ?? null,
        from_ms: args?.fromMs ?? null,
        to_ms: args?.toMs ?? null,
        limit: args?.limit ?? null,
        offset: args?.offset ?? null,
      },
    }),

  // skills
  listSkills: () => invoke<SkillInfo[]>("list_skills"),
  getSkill: (id: string) => invoke<SkillInfo>("get_skill", { id }),
  listEnabledSkills: () => invoke<SkillInfo[]>("list_enabled_skills"),
  setSkillEnabled: (id: string, enabled: boolean) =>
    invoke<Settings>("set_skill_enabled", { args: { id, enabled } }),
  importSkill: (path: string) =>
    invoke<SkillInfo>("import_skill", { args: { path } }),
  uninstallSkill: (id: string) => invoke<void>("uninstall_skill", { id }),
  getSkillsDir: () => invoke<string>("get_skills_dir"),
};

export function srcOf(absPath: string | null | undefined): string {
  if (!absPath) return "";
  return convertFileSrc(absPath);
}

export type { MessageAbs };
