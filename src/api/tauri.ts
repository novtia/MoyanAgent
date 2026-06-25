import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Role } from "../store/roleState";
import type {
  AgentDefinitionInfo,
  AgentSummary,
  AttachmentDraft,
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
  ProjectTextFile,
  SessionSearchResult,
  SessionSummary,
  SessionWithMessagesAbs,
  TokenUsageEventRow,
  TokenUsageSummary,
  Session,
  Settings,
  SettingsPatch,
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
    invoke<string[]>("fetch_provider_models", {
      args: { sdk, endpoint, apiKey },
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
  toggleDevtools: () => invoke<void>("toggle_devtools"),

  // sessions
  listSessions: () => invoke<SessionSummary[]>("list_sessions"),
  searchSessions: (query: string, limit = 20) =>
    invoke<SessionSearchResult[]>("search_sessions", { query, limit }),
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
  setSessionModel: (id: string, model: string, contextWindow: number | null) =>
    invoke<void>("set_session_model", {
      args: { id, model, contextWindow },
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
      thinking_enabled?: boolean | null;
      thinking_effort?: string | null;
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
      thinking_enabled?: boolean | null;
      thinking_effort?: string | null;
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

  // project / session transfer
  exportProjectsArchive: (projectIds: string[], destPath: string) =>
    invoke<void>("export_projects_archive", { projectIds, destPath }),

  exportSessionArchive: (sessionId: string, destPath: string) =>
    invoke<void>("export_session_archive", { sessionId, destPath }),

  importArchive: (archivePath: string) =>
    invoke<ImportResult>("import_archive", { archivePath }),

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

  deleteProjectPath: (sessionId: string, path: string) =>
    invoke<void>("delete_project_path", { sessionId, path }),

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
};

export function srcOf(absPath: string | null | undefined): string {
  if (!absPath) return "";
  return convertFileSrc(absPath);
}

export type { MessageAbs };
