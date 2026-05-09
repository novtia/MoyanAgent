export interface Settings {
  api_key: string;
  endpoint: string;
  model: string;
  default_aspect_ratio: string;
  default_image_size: string;
  system_prompt: string;
  temperature: number | null;
  top_p: number | null;
  max_tokens: number | null;
  frequency_penalty: number | null;
  presence_penalty: number | null;
  history_turns: number;
}

export interface SettingsPatch {
  api_key?: string;
  endpoint?: string;
  model?: string;
  default_aspect_ratio?: string;
  default_image_size?: string;
  system_prompt?: string;
  temperature?: number | null;
  top_p?: number | null;
  max_tokens?: number | null;
  frequency_penalty?: number | null;
  presence_penalty?: number | null;
  history_turns?: number;
}

export interface Session {
  id: string;
  title: string;
  model: string | null;
  created_at: number;
  updated_at: number;
}

export interface SessionSummary {
  id: string;
  title: string;
  model: string | null;
  updated_at: number;
  message_count: number;
}

export interface ImageRefAbs {
  id: string;
  role: "input" | "output" | "edited" | string;
  rel_path: string;
  thumb_rel_path: string | null;
  abs_path: string;
  thumb_abs_path: string | null;
  mime: string;
  width: number | null;
  height: number | null;
  bytes: number | null;
  ord: number;
}

export interface MessageAbs {
  id: string;
  session_id: string;
  role: "user" | "assistant" | "error" | string;
  text: string | null;
  params: { aspect_ratio?: string; image_size?: string } | null;
  created_at: number;
  images: ImageRefAbs[];
}

export interface SessionWithMessagesAbs {
  session: Session;
  messages: MessageAbs[];
}

export interface AttachmentDraft {
  image_id: string;
  rel_path: string;
  thumb_rel_path: string | null;
  abs_path: string;
  thumb_abs_path: string | null;
  mime: string;
  width: number | null;
  height: number | null;
  bytes: number | null;
}

export type EditOp =
  | { type: "crop"; x: number; y: number; width: number; height: number }
  | { type: "resize"; width: number; height: number }
  | { type: "rotate"; degrees: number }
  | { type: "flip"; horizontal: boolean }
  | { type: "apply_mask"; mask_png_base64: string };

export interface GenerateResult {
  user_message: MessageAbs;
  assistant_message: MessageAbs;
}
