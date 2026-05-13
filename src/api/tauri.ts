import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type {
  AttachmentDraft,
  EditOp,
  GenerateResult,
  ImageRefAbs,
  LlmModelCatalog,
  MessageAbs,
  ModelParamSettings,
  SessionSearchResult,
  SessionSummary,
  SessionWithMessagesAbs,
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

  // app info
  getAppInfo: () =>
    invoke<{
      version: string;
      data_dir: string;
      db_path: string;
      sessions_dir: string;
    }>("get_app_info"),
  openPath: (path: string) => invoke<void>("open_path", { path }),

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
  deleteSession: (id: string) => invoke<void>("delete_session", { id }),
  loadSession: (id: string) =>
    invoke<SessionWithMessagesAbs>("load_session", { id }),
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

  // local editing
  editImage: (imageId: string, op: EditOp) =>
    invoke<ImageRefAbs>("edit_image", { args: { image_id: imageId, op } }),

  // export
  exportImage: (imageId: string, destPath: string) =>
    invoke<void>("export_image", { args: { image_id: imageId, dest_path: destPath } }),
};

export function srcOf(absPath: string | null | undefined): string {
  if (!absPath) return "";
  return convertFileSrc(absPath);
}

export type { MessageAbs };
