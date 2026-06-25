export interface ModelParamSettings {
  temperature: number | null;
  top_p: number | null;
  max_tokens: number | null;
  frequency_penalty: number | null;
  presence_penalty: number | null;
  /** When true, enables extended reasoning where the SDK supports it. */
  thinking_enabled: boolean | null;
  /** e.g. low / medium / high / max — forwarded as OpenAI `reasoning_effort` or Claude `output_config.effort`. */
  thinking_effort: string | null;
}

export interface ModelServiceModel {
  id: string;
  name: string;
  group: string;
  capabilities: string[];
  /** Max context window (tokens); omit or null when unknown. */
  context_window?: number | null;
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

/** SDK adapter metadata + default models (from app catalog / DB). */
export interface ProviderSdkConfig {
  id: string;
  label: string;
  description: string;
  defaultName: string;
  defaultEndpoint: string;
  endpointPlaceholder: string;
  endpointHint: string;
  apiKeyPlaceholder: string;
  apiKeyHint: string;
  modelIdPlaceholder: string;
  modelIdHint: string;
  models: ModelServiceModel[];
}

export interface LlmModelCatalog {
  providerSdkOptions: ProviderSdkConfig[];
  builtinProviderPresets: ModelProvider[];
}

export interface Settings {
  api_key: string;
  endpoint: string;
  model: string;
  active_provider_id: string;
  model_services: ModelProvider[];
  /** Provider id of the quick model used for lightweight tasks (e.g. session title generation). */
  quick_model_provider_id: string;
  /** Model id of the quick model. */
  quick_model: string;
  default_aspect_ratio: string;
  default_image_size: string;
  /** Global default for the composer thinking toggle (reasoning models only). */
  default_thinking_enabled: boolean;
  /** Global default reasoning effort; empty string means provider default (high). */
  default_thinking_effort: string;
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
  quick_model_provider_id?: string;
  quick_model?: string;
  default_aspect_ratio?: string;
  default_image_size?: string;
  default_thinking_enabled?: boolean;
  default_thinking_effort?: string;
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
  /** Context window limit (tokens); null defers to model/catalog. */
  context_window: number | null;
  /** Cumulative usage (tokens) tracked for this session. */
  context_window_used: number;
  /** Main chat agent: `general-purpose` (Agent) or `Plan` (read-only planning). */
  agent_type: string;
  /**
   * Ordered agent flow chain. When set and non-empty the turn runs as a
   * streaming pipeline through each node in order; otherwise a single
   * `agent_type` run applies. Each entry is either a bare agent_type string or
   * a node carrying per-node config overrides (see {@link ChainEntry}).
   */
  agent_chain: ChainEntry[] | null;
  /** Project this session belongs to, if any. */
  project_id: string | null;
  created_at: number;
  updated_at: number;
}

/**
 * Per-node config overrides applied to a single position in an agent flow
 * chain. Each field is optional; an omitted field keeps the agent's default.
 * These overrides live only inside the chain and never mutate the global
 * built-in / custom agent definition.
 */
export interface NodeOverrides {
  system_prompt?: string;
  model?: string | null;
  tools?: string[];
}

/** A chain node carrying per-node overrides. */
export interface ChainNode {
  agent_type: string;
  overrides?: NodeOverrides;
}

/**
 * Wire form of a chain entry: a bare `agent_type` string (no overrides) or a
 * {@link ChainNode} object. The backend serialises nodes without overrides
 * back to bare strings, so a loaded chain may mix both shapes.
 */
export type ChainEntry = string | ChainNode;

/** Full resolved configuration of an agent (from `get_agent_definition`). */
export interface AgentDefinitionInfo {
  agent_type: string;
  when_to_use: string;
  system_prompt: string;
  model: string | null;
  tools: string[];
  background: boolean;
  passthrough_output: boolean;
}

/** A user-defined sub-agent saved globally and reusable across sessions. */
export interface CustomAgent {
  agent_type: string;
  name: string;
  when_to_use: string;
  system_prompt: string;
  model: string | null;
  /** Allowed tool names. Empty means full tool access. */
  tools: string[];
  created_at: number;
  updated_at: number;
}

/** Summary of a built-in / registered agent definition (from `list_agents`). */
export interface AgentSummary {
  agent_type: string;
  when_to_use: string;
  background: boolean;
  tools: string[];
  disallowed_tools: string[];
}

export interface SessionSummary {
  id: string;
  title: string;
  model: string | null;
  system_prompt: string;
  history_turns: number;
  llm_params: ModelParamSettings;
  context_window: number | null;
  context_window_used: number;
  agent_type: string;
  updated_at: number;
  message_count: number;
  project_id: string | null;
}

export interface Project {
  id: string;
  name: string;
  path: string | null;
  sort_order: number;
  /** Shared system prompt applied to all sessions in this project. */
  system_prompt: string;
  /** Number of history turns for project sessions. */
  history_turns: number;
  /** Shared LLM sampling params for project sessions. */
  llm_params: ModelParamSettings;
  /** Optional context window override (tokens) for project sessions. */
  context_window: number | null;
  /**
   * Shared agent flow chain applied to every session in this project. When set
   * and non-empty, conversations under the project run as the same multi-agent
   * pipeline; null means single-agent runs.
   */
  agent_chain: string[] | null;
  created_at: number;
  updated_at: number;
}

export interface ProjectDirEntry {
  name: string;
  path: string;
  isDir: boolean;
}

/** Text file payload from `read_project_file` (encoding preserved for save). */
export interface ProjectTextFile {
  text: string;
  encoding: string;
  hadBom: boolean;
}

export const DEFAULT_TEXT_ENCODING = "utf-8";

export interface SessionSearchResult extends SessionSummary {
  match_message_id: string | null;
  match_role: "user" | "assistant" | "error" | string | null;
  match_text: string | null;
  match_created_at: number | null;
  match_count: number;
  title_match: boolean;
  project_id: string | null;
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

/**
 * One inline content block of an assistant message.
 *
 * Assistant messages are an *ordered list* of blocks so that thinking,
 * text, and tool calls render in the exact order they streamed in. The
 * agent loop can produce multiple thinking/text/tool blocks per
 * message (one set per inner turn) and the renderer must preserve that
 * interleaving — see the design note in
 * `docs/工具调用_ui_渲染.plan.md`.
 */
export type AssistantBlock =
  | { type: "thinking"; content: string }
  | { type: "text"; content: string }
  | {
      type: "tool_use";
      id: string;
      tool: string;
      input: unknown;
      status: "pending" | "success" | "error";
      output?: unknown;
      is_error?: boolean;
      /**
       * True while the tool's input arguments are still streaming in (OpenAI
       * tool-call deltas). Cleared once the terminal `tool_use` event arrives
       * with the fully-parsed input. Used to drive live cursors / spinners and
       * to defer side-effects (e.g. opening the reader) until input is final.
       */
      streaming?: boolean;
    }
  | {
      /**
       * Marks the start of an agent flow stage. Emitted before each agent in a
       * multi-agent chain so the renderer can show stage separators
       * (main -> state-machine -> fixer -> ...).
       */
      type: "agent_stage";
      agent_type: string;
      name?: string;
      index?: number;
    };

export interface MessageAbs {
  id: string;
  session_id: string;
  role: "user" | "assistant" | "error" | string;
  text: string | null;
  params: {
    aspect_ratio?: string;
    image_size?: string;
    thinking_content?: string | null;
    /**
     * Ordered, streamed inline blocks for assistant messages. Newer
     * messages always populate this; older messages (created before the
     * blocks model existed) only carry `text` / `thinking_content` and
     * the renderer falls back to the legacy single-segment view.
     */
    blocks?: AssistantBlock[];
    usage?: {
      prompt_tokens?: number | null;
      completion_tokens?: number | null;
      total_tokens?: number | null;
      last_prompt_tokens?: number | null;
    };
  } | null;
  created_at: number;
  images: ImageRefAbs[];
}

export interface SessionWithMessagesAbs {
  session: Session;
  messages: MessageAbs[];
}

export interface TokenUsageEventRow {
  id: string;
  created_at: number;
  event_kind: string;
  session_id?: string | null;
  correlation_id?: string | null;
  message_id?: string | null;
  agent_id?: string | null;
  agent_type?: string | null;
  model?: string | null;
  provider?: string | null;
  turn_index?: number | null;
  tool_name?: string | null;
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  output_chars?: number | null;
  output_bytes?: number | null;
  is_error: boolean;
  metadata_json?: string | null;
}

export interface ModelUsageRow {
  model: string;
  provider?: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  event_count: number;
}

export interface TokenUsageSummary {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  api_call_count: number;
  tool_call_count: number;
  turn_summary_count: number;
  by_model: ModelUsageRow[];
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

export interface ImportResult {
  projects_imported: number;
  sessions_imported: number;
  messages_imported: number;
}
