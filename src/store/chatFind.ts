import { create } from "zustand";
import { api } from "../api/tauri";
import type { SessionSearchHit } from "../types";

interface ChatFindActivateOpts {
  sessionId: string;
  query: string;
  preferredMessageId?: string;
}

interface ChatFindStore {
  open: boolean;
  sessionId: string | null;
  query: string;
  hits: SessionSearchHit[];
  hitIndex: number;
  loading: boolean;
  listOpen: boolean;
  activate: (opts: ChatFindActivateOpts) => Promise<void>;
  nextMatch: () => void;
  prevMatch: () => void;
  goToHit: (index: number) => void;
  close: () => void;
  setListOpen: (open: boolean) => void;
  /** Active hit message id when find bar is open. */
  activeMessageId: () => string | null;
}

function focusMessage(messageId: string) {
  window.dispatchEvent(
    new CustomEvent("atelier:focus-message", {
      detail: { messageId, persist: true },
    }),
  );
}

function resolveHitIndex(
  hits: SessionSearchHit[],
  preferredMessageId?: string,
): number {
  if (hits.length === 0) return -1;
  if (preferredMessageId) {
    const idx = hits.findIndex((h) => h.message_id === preferredMessageId);
    if (idx >= 0) return idx;
  }
  return 0;
}

export const useChatFind = create<ChatFindStore>((set, get) => ({
  open: false,
  sessionId: null,
  query: "",
  hits: [],
  hitIndex: -1,
  loading: false,
  listOpen: false,

  activate: async ({ sessionId, query, preferredMessageId }) => {
    const q = query.trim();
    if (!q) {
      get().close();
      return;
    }
    set({
      open: true,
      sessionId,
      query: q,
      loading: true,
      hits: [],
      hitIndex: -1,
      listOpen: false,
    });
    try {
      const hits = await api.searchSessionHits(sessionId, q, 200);
      if (get().sessionId !== sessionId || get().query !== q) return;
      const hitIndex = resolveHitIndex(hits, preferredMessageId);
      set({ hits, hitIndex, loading: false });
      if (hitIndex >= 0) {
        const hit = hits[hitIndex];
        if (hit) focusMessage(hit.message_id);
      }
    } catch (err) {
      console.warn(err);
      if (get().sessionId !== sessionId) return;
      set({ hits: [], hitIndex: -1, loading: false });
    }
  },

  nextMatch: () => {
    const { hits, hitIndex } = get();
    if (hits.length === 0) return;
    const next = hitIndex < 0 ? 0 : (hitIndex + 1) % hits.length;
    set({ hitIndex: next });
    const hit = hits[next];
    if (hit) focusMessage(hit.message_id);
  },

  prevMatch: () => {
    const { hits, hitIndex } = get();
    if (hits.length === 0) return;
    const prev = hitIndex <= 0 ? hits.length - 1 : hitIndex - 1;
    set({ hitIndex: prev });
    const hit = hits[prev];
    if (hit) focusMessage(hit.message_id);
  },

  goToHit: (index) => {
    const { hits } = get();
    if (index < 0 || index >= hits.length) return;
    set({ hitIndex: index });
    const hit = hits[index];
    if (hit) focusMessage(hit.message_id);
  },

  close: () => {
    set({
      open: false,
      sessionId: null,
      query: "",
      hits: [],
      hitIndex: -1,
      loading: false,
      listOpen: false,
    });
  },

  setListOpen: (listOpen) => set({ listOpen }),

  activeMessageId: () => {
    const { open, hits, hitIndex } = get();
    if (!open || hitIndex < 0 || hitIndex >= hits.length) return null;
    return hits[hitIndex]?.message_id ?? null;
  },
}));
