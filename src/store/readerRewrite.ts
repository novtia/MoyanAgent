import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import type { ModelProvider } from "../types";
import { normalizeReaderPath } from "./reader";

export type RewriteMode = "rewrite" | "expand" | "pipeline";
export type RewriteStage = "rewrite" | "expand";

export interface RewriteRun {
  view: "rewrite" | "files";
  token: string;
  mode: RewriteMode;
  running: boolean;
  stage: RewriteStage | null;
  index: number;
  total: number;
  text: string;
  thinking: string;
  error: string | null;
  cancelled: boolean;
}

interface ProgressEvent {
  path: string;
  clientToken: string;
  kind: "stage" | "delta" | "thinking" | "done" | "error" | "cancelled";
  stage?: string | null;
  index?: number | null;
  total?: number | null;
  text?: string | null;
  message?: string | null;
}

export interface RewriteModelOption {
  providerId: string;
  modelId: string;
  name: string;
  sdk: string;
}

export function rewriteModelKey(providerId: string, modelId: string) {
  return `${providerId}\u0000${modelId}`;
}

export function rewriteModelOptions(providers: ModelProvider[]): RewriteModelOption[] {
  const out: RewriteModelOption[] = [];
  for (const provider of providers) {
    if (provider.enabled === false) continue;
    for (const model of provider.models) {
      if (!model.capabilities?.includes("rewrite")) continue;
      out.push({
        providerId: provider.id,
        modelId: model.id,
        name: model.name || model.id,
        sdk: provider.sdk ?? "",
      });
    }
  }
  return out;
}

function blankRun(mode: RewriteMode, token: string): RewriteRun {
  return {
    view: "rewrite",
    token,
    mode,
    running: true,
    stage: null,
    index: 0,
    total: 0,
    text: "",
    thinking: "",
    error: null,
    cancelled: false,
  };
}

let listening = false;

export function ensureRewriteEvents() {
  if (listening) return;
  listening = true;
  void listen<ProgressEvent>("rewrite://progress", (event) => {
    useReaderRewrite.getState().apply(event.payload);
  });
}

interface ReaderRewriteState {
  mode: RewriteMode;
  modelKey: string;
  byPath: Record<string, RewriteRun>;
  setMode: (mode: RewriteMode) => void;
  setModelKey: (key: string) => void;
  setView: (path: string, view: "rewrite" | "files") => void;
  begin: (path: string, mode: RewriteMode) => string;
  apply: (event: ProgressEvent) => void;
  fail: (path: string, token: string, message: string) => void;
}

export const useReaderRewrite = create<ReaderRewriteState>((set, get) => ({
  mode: "rewrite",
  modelKey: "",
  byPath: {},
  setMode: (mode) => set({ mode }),
  setModelKey: (modelKey) => set({ modelKey }),
  setView: (path, view) => {
    const key = normalizeReaderPath(path);
    const current = get().byPath[key];
    if (!current) return;
    set({ byPath: { ...get().byPath, [key]: { ...current, view } } });
  },
  begin: (path, mode) => {
    const key = normalizeReaderPath(path);
    const token = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    set({ byPath: { ...get().byPath, [key]: blankRun(mode, token) } });
    return token;
  },
  fail: (path, token, message) => {
    const key = normalizeReaderPath(path);
    const current = get().byPath[key];
    if (!current || current.token !== token) return;
    if (!current.running && current.error) return;
    set({
      byPath: {
        ...get().byPath,
        [key]: { ...current, running: false, error: message },
      },
    });
  },
  apply: (event) => {
    const key = normalizeReaderPath(event.path);
    const current = get().byPath[key];
    if (!current || current.token !== event.clientToken) return;
    if (event.kind === "stage") {
      const stage: RewriteStage | null =
        event.stage === "expand" || event.stage === "rewrite" ? event.stage : current.stage;
      set({
        byPath: {
          ...get().byPath,
          [key]: {
            ...current,
            running: true,
            view: "rewrite",
            stage,
            index: event.index ?? current.index,
            total: event.total ?? current.total,
            error: null,
            cancelled: false,
          },
        },
      });
      return;
    }
    if (event.kind === "thinking") {
      const piece = event.text ?? "";
      if (!piece) return;
      const gap = current.thinking ? "\n\n" : "";
      set({
        byPath: {
          ...get().byPath,
          [key]: { ...current, thinking: current.thinking + gap + piece },
        },
      });
      return;
    }
    if (event.kind === "delta") {
      set({
        byPath: {
          ...get().byPath,
          [key]: { ...current, text: current.text + (event.text ?? "") },
        },
      });
      return;
    }
    if (event.kind === "done") {
      set({
        byPath: {
          ...get().byPath,
          [key]: {
            ...current,
            running: false,
            text: event.text ?? current.text,
            error: null,
            cancelled: false,
          },
        },
      });
      return;
    }
    if (event.kind === "error") {
      set({
        byPath: {
          ...get().byPath,
          [key]: {
            ...current,
            running: false,
            error: event.message?.trim() || "改文失败",
          },
        },
      });
      return;
    }
    if (event.kind === "cancelled") {
      set({
        byPath: {
          ...get().byPath,
          [key]: { ...current, running: false, cancelled: true },
        },
      });
    }
  },
}));
