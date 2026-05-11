export interface ModelParamSettings {
  temperature: number | null;
  top_p: number | null;
  max_tokens: number | null;
  frequency_penalty: number | null;
  presence_penalty: number | null;
}

export interface ModelServiceModel {
  id: string;
  name: string;
  group: string;
  capabilities: string[];
}

export type ModelProviderSdk =
  | "openai"
  | "openai-responses"
  | "gemini"
  | "claude"
  | "grok"
  | "ark-images";

export interface ModelProvider {
  id: string;
  name: string;
  /** Backend SDK adapter. Defaults to OpenAI chat completions when omitted. */
  sdk?: ModelProviderSdk | (string & {});
  avatar?: string;
  endpoint: string;
  api_key: string;
  /** When false, hidden from chat model picker and not used for requests. Default true. */
  enabled?: boolean;
  models: ModelServiceModel[];
}

export interface Settings {
  api_key: string;
  endpoint: string;
  model: string;
  active_provider_id: string;
  model_services: ModelProvider[];
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
  active_provider_id?: string;
  model_services?: ModelProvider[];
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
  system_prompt: string;
  history_turns: number;
  llm_params: ModelParamSettings;
  created_at: number;
  updated_at: number;
}

export interface SessionSummary {
  id: string;
  title: string;
  model: string | null;
  system_prompt: string;
  history_turns: number;
  llm_params: ModelParamSettings;
  updated_at: number;
  message_count: number;
}

export interface SessionSearchResult extends SessionSummary {
  match_message_id: string | null;
  match_role: "user" | "assistant" | "error" | string | null;
  match_text: string | null;
  match_created_at: number | null;
  match_count: number;
  title_match: boolean;
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
  params: {
    aspect_ratio?: string;
    image_size?: string;
    usage?: {
      prompt_tokens?: number | null;
      completion_tokens?: number | null;
      total_tokens?: number | null;
    };
  } | null;
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
