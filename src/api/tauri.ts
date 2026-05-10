import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type {
  AttachmentDraft,
  EditOp,
  GenerateResult,
  ImageRefAbs,
  MessageAbs,
  SessionSearchResult,
  SessionSummary,
  SessionWithMessagesAbs,
  Session,
  Settings,
  SettingsPatch,
} from "../types";

export const api = {
  // settings
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<Settings>("update_settings", { patch }),

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
  updateSessionConfig: (id: string, systemPrompt: string, historyTurns: number) =>
    invoke<void>("update_session_config", {
      id,
      systemPrompt,
      historyTurns,
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
  generateImage: (req: {
    session_id: string;
    prompt: string;
    attachment_ids: string[];
    aspect_ratio: string;
    image_size: string;
  }) => invoke<GenerateResult>("generate_image", { req }),
  regenerateImage: (req: {
    session_id: string;
    user_message_id: string;
    aspect_ratio: string;
    image_size: string;
  }) => invoke<GenerateResult>("regenerate_image", { req }),
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
