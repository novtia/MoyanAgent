import { create } from "zustand";
import type {
  ChainEntry,
  MessageAbs,
  ModelParamSettings,
  SessionWithMessagesAbs,
} from "../../types";
import { api } from "../../api/tauri";
import {
  agentTypeFromComposerMode,
  composerModeFromAgentType,
} from "../../config/chatMode";
import { useRoleState } from "../roleState";
import { useReader, syncPendingDiffsForSession } from "../reader";
import { playNotifySound } from "../notifySound";
import {
  type PendingAskUser,
  askUserCustomText,
  firstUnansweredAskUserIndex,
  flushAskUserPrompt,
  formatAskUserItems,
  formatAskUserReply,
  questionKey,
} from "../../components/chat/askUser";
import {
  fileToBytes,
  isVideoModel,
  makePendingAttachment,
  matchesImageRole,
  maxBytesForMime,
  pathLabel,
  stripMediaMentionTokens,
  uploadMime,
} from "./attachmentHelpers";
import { ACCEPT_TYPES, MAX_FILES } from "./constants";
import {
  askUserPendingBySession,
  composerDrafts,
  roleStateScopeForSession,
  saveComposerDraft,
} from "./drafts";
import {
  bumpGenerationEpoch,
  effectiveSessionForLog,
  generationFlights,
  getGenerationEpoch,
  isGenerationCancelled,
  persistPartialStreamIfAny,
  refreshReaderAfterFileRollback,
  trackGenerationFlight,
  waitForGenerationIdle,
} from "./generation";
import { ensureGenerationStreamListener } from "./listeners";
import { activeModelCapabilities } from "./modelCapabilities";
import {
  applyStreamBufferToMessages,
  cancelStreamFlushRaf,
  cancellingSessions,
  freezeStreamingUi,
  streamingBuffers,
} from "./streaming";
import type { PendingAttachmentDraft, SessionStore } from "./types";
import {
  coerceVideoModeForMedia,
  resolvedMediaRole,
  validateVideoComposer,
} from "./videoMode";

export const useSession = create<SessionStore>((set, get) => {

  const setSessionBusy = (sessionId: string, busy: boolean) => {
    set((state) => {
      const busyBySession = { ...state.busyBySession };
      const finishedBySession = { ...state.finishedBySession };
      if (busy) {
        busyBySession[sessionId] = true;
        // A new run supersedes any lingering "task complete" reminder.
        delete finishedBySession[sessionId];
      } else {
        delete busyBySession[sessionId];
      }
      const generationPhaseBySession = {
        ...state.generationPhaseBySession,
      };
      if (!busy) delete generationPhaseBySession[sessionId];
      return {
        busyBySession,
        finishedBySession,
        generationPhaseBySession,
        busy: state.activeId ? !!busyBySession[state.activeId] : false,
      };
    });
  };

  /** Flag a session's generation as finished in the background (reminder). */
  const markSessionFinished = (sessionId: string) => {
    set((state) => ({
      finishedBySession: { ...state.finishedBySession, [sessionId]: true },
    }));
  };

  const updateActiveSession = (
    sessionId: string,
    update: (active: SessionWithMessagesAbs) => SessionWithMessagesAbs,
  ) => {
    const active = get().active;
    if (!active || get().activeId !== sessionId) return;
    set({ active: update(active) });
  };

  /** Replace active chat with server truth (avoids duplicate/stale merges after async gen or tab switches). */
  const reloadActiveSessionIfViewing = async (sessionId: string) => {
    // Generation is complete — always discard the streaming buffer, even when
    // the user is viewing another session. A leftover buffer would otherwise
    // be picked up by the next generation in this session and render the
    // previous assistant reply as part of the new streaming message.
    cancelStreamFlushRaf(sessionId);
    streamingBuffers.delete(sessionId);
    if (get().activeId !== sessionId) return;
    try {
      const data = await api.loadSession(sessionId);
      set({
        active: data,
        composer: {
          ...get().composer,
          chatMode: composerModeFromAgentType(data.session.agent_type),
        },
      });
    } catch (e) {
      console.warn(e);
    }
  };

  /** Cancel in-flight generation and wait for the invoke to settle. */
  const abortInFlightGeneration = async (sessionId: string) => {
    const busy = !!get().busyBySession[sessionId];
    const hasFlight = generationFlights.has(sessionId);
    if (!busy && !hasFlight) return;

    if (busy && !cancellingSessions.has(sessionId)) {
      cancellingSessions.add(sessionId);
      setSessionBusy(sessionId, false);
      freezeStreamingUi(sessionId);
      void api.cancelGeneration(sessionId).catch((e) => {
        console.warn("[atelier] cancel_generation failed", e);
      });
    }
    bumpGenerationEpoch(sessionId);
    await waitForGenerationIdle(sessionId);
  };

  /** Message ids after `idx` that exist in the DB (skip optimistic tmp rows). */
  const persistedIdsAfter = (messages: MessageAbs[], idx: number) =>
    messages
      .slice(idx + 1)
      .filter((msg) => !msg.id.startsWith("tmp-"))
      .map((msg) => msg.id);

  return ({
  sessions: [],
  activeId: null,
  active: null,
  busy: false,
  busyBySession: {},
  finishedBySession: {},
  generationPhaseBySession: {},
  composer: {
    prompt: "",
    mentions: [],
    attachments: [],
    pendingAttachments: [],
    aspectRatio: "auto",
    imageSize: "auto",
    videoMode: "text",
    videoDuration: 5,
    videoResolution: "720p",
    generateAudio: true,
    watermark: false,
    thinkingEnabled: false,
    thinkingEffort: "",
    chatMode: "agent",
  },
  pendingAskUser: null,

  refreshList: async () => {
    const list = await api.listSessions();
    set({ sessions: list });
  },

  createNew: async () => {
    const s = await api.createSession();
    await get().refreshList();
    await get().switchTo(s.id);
    return s.id;
  },

  switchTo: async (id) => {
    // Save the current session's composer draft before switching away.
    const currentId = get().activeId;
    if (currentId && currentId !== id) {
      saveComposerDraft(currentId, get().composer);
      const pending = get().pendingAskUser;
      if (pending && pending.sessionId === currentId) {
        const flushed = flushAskUserPrompt(pending, get().composer.prompt);
        askUserPendingBySession.set(currentId, flushed);
      }
    }

    const data = await api.loadSession(id);
    const isBusy = !!get().busyBySession[id];

    // Opening a session acknowledges any background "task complete" reminder.
    if (get().finishedBySession[id]) {
      set((state) => {
        const finishedBySession = { ...state.finishedBySession };
        delete finishedBySession[id];
        return { finishedBySession };
      });
    }

    // If this session is still generating (or already has a live buffer),
    // restore buffered streaming content accumulated while away.
    let messagesWithBuffer = data.messages;
    const buf = streamingBuffers.get(id);
    if ((isBusy || buf) && buf && buf.blocks.length > 0) {
      messagesWithBuffer = applyStreamBufferToMessages(data.messages, id, buf);
    }

    // Restore draft for the target session, falling back to empty defaults.
    const draft = composerDrafts.get(id);
    const pendingAsk = askUserPendingBySession.get(id) ?? null;
    const askCustom =
      pendingAsk != null
        ? askUserCustomText(pendingAsk, pendingAsk.activeIndex)
        : null;
    set({
      activeId: id,
      active: { ...data, messages: messagesWithBuffer },
      busy: isBusy,
      pendingAskUser: pendingAsk,
      composer: {
        ...get().composer,
        prompt: askCustom ?? draft?.prompt ?? "",
        mentions: draft?.mentions ?? [],
        attachments: draft?.attachments ?? [],
        pendingAttachments: [],
        aspectRatio: draft?.aspectRatio ?? get().composer.aspectRatio,
        imageSize: draft?.imageSize ?? get().composer.imageSize,
        videoMode: draft?.videoMode ?? get().composer.videoMode,
        videoDuration: draft?.videoDuration ?? get().composer.videoDuration,
        videoResolution: draft?.videoResolution ?? get().composer.videoResolution,
        generateAudio: draft?.generateAudio ?? get().composer.generateAudio,
        watermark: draft?.watermark ?? get().composer.watermark,
        thinkingEnabled: data.session.llm_params?.thinking_enabled ?? false,
        thinkingEffort: data.session.llm_params?.thinking_effort ?? "",
        chatMode: composerModeFromAgentType(data.session.agent_type),
      },
    });
    void useRoleState.getState().loadLatest(id, roleStateScopeForSession(id));
    useReader.getState().bindSession(id);
    void syncPendingDiffsForSession(id);
  },

  rename: async (id, title) => {
    await api.renameSession(id, title);
    await get().refreshList();
    if (get().activeId === id && get().active) {
      const a = get().active!;
      set({ active: { ...a, session: { ...a.session, title } } });
    }
  },

  updateConfig: async (id, systemPrompt, historyTurns, llmParams) => {
    await api.updateSessionConfig(id, systemPrompt, historyTurns, llmParams);
    await get().refreshList();
    if (get().activeId === id && get().active) {
      const a = get().active!;
      set({
        active: {
          ...a,
          session: {
            ...a.session,
            system_prompt: systemPrompt,
            history_turns: historyTurns,
            llm_params: llmParams,
          },
        },
      });
    }
  },

  remove: async (id) => {
    const parentId =
      get().activeId === id
        ? get().active?.session.parent_session_id ?? null
        : null;
    await api.deleteSession(id);
    askUserPendingBySession.delete(id);
    composerDrafts.delete(id);
    streamingBuffers.delete(id);
    if (get().activeId === id) {
      if (parentId) {
        await get().switchTo(parentId);
      } else {
        set({ activeId: null, active: null, busy: false, pendingAskUser: null });
      }
    }
    await get().refreshList();
  },

  ensureActive: async () => {
    const cur = get().activeId;
    if (cur) return cur;
    return await get().createNew();
  },

  reloadActiveSession: async () => {
    const id = get().activeId;
    if (!id) return;
    try {
      const data = await api.loadSession(id);
      const state = get();
      // A settings-only reload (for example, after switching models) can race
      // with an in-flight stream. Keep the optimistic/streaming rows until the
      // generation completion path replaces them with the persisted messages.
      // Also ignore a stale response if the user switched sessions meanwhile.
      if (state.activeId !== id) return;
      const preserveLiveMessages =
        !!state.busyBySession[id] && state.active?.session.id === id;
      set({
        active: preserveLiveMessages
          ? { ...data, messages: state.active!.messages }
          : data,
        composer: {
          ...get().composer,
          chatMode: composerModeFromAgentType(data.session.agent_type),
        },
      });
    } catch (e) {
      console.warn(e);
    }
  },

  dismissFinished: (id) => {
    set((state) => {
      if (!state.finishedBySession[id]) return state;
      const finishedBySession = { ...state.finishedBySession };
      delete finishedBySession[id];
      return { finishedBySession };
    });
  },

  setPrompt: (s) => set({ composer: { ...get().composer, prompt: s } }),
  setMentions: (paths) => set({ composer: { ...get().composer, mentions: paths } }),
  setAspectRatio: (s) => set({ composer: { ...get().composer, aspectRatio: s } }),
  setImageSize: (s) => set({ composer: { ...get().composer, imageSize: s } }),
  setVideoMode: (mode) => set({ composer: { ...get().composer, videoMode: mode } }),
  setVideoDuration: (duration) =>
    set({ composer: { ...get().composer, videoDuration: duration } }),
  setVideoResolution: (resolution) =>
    set({ composer: { ...get().composer, videoResolution: resolution } }),
  setGenerateAudio: (enabled) =>
    set({ composer: { ...get().composer, generateAudio: enabled } }),
  setWatermark: (enabled) =>
    set({ composer: { ...get().composer, watermark: enabled } }),
  setThinkingEnabled: (on) =>
    set({ composer: { ...get().composer, thinkingEnabled: on } }),
  setThinkingEffort: (effort) =>
    set({ composer: { ...get().composer, thinkingEffort: effort } }),

  persistComposerThinking: async (sessionId) => {
    const a = get().active;
    if (!a || a.session.id !== sessionId) return;
    const c = get().composer;
    const canReason = activeModelCapabilities().includes("reasoning");
    const enabled = canReason ? c.thinkingEnabled : false;
    const effort =
      canReason && c.thinkingEnabled && c.thinkingEffort.trim()
        ? c.thinkingEffort.trim()
        : null;
    const cur = a.session.llm_params;
    if ((cur.thinking_enabled ?? false) === enabled && (cur.thinking_effort ?? null) === effort) {
      return;
    }
    const llm: ModelParamSettings = {
      ...cur,
      thinking_enabled: enabled,
      thinking_effort: effort,
    };
    try {
      await api.updateSessionConfig(
        sessionId,
        a.session.system_prompt,
        a.session.history_turns,
        llm,
      );
      const now = get().active;
      if (now && now.session.id === sessionId) {
        set({ active: { ...now, session: { ...now.session, llm_params: llm } } });
      }
    } catch (e) {
      console.warn(e);
    }
  },

  setChatMode: async (mode) => {
    const id = get().activeId;
    if (!id) {
      set({ composer: { ...get().composer, chatMode: mode } });
      return;
    }
    try {
      await api.setSessionAgentType(id, agentTypeFromComposerMode(mode));
      await get().refreshList();
      await get().reloadActiveSession();
    } catch (e) {
      console.warn(e);
    }
  },

  setAgentChain: async (chain) => {
    const id = get().activeId;
    if (!id) return;
    const cleaned = chain
      .map((e): ChainEntry => {
        if (typeof e === "string") return e.trim();
        const at = e.agent_type.trim();
        const ov = e.overrides;
        const hasOv =
          !!ov &&
          (ov.system_prompt !== undefined ||
            ov.model !== undefined ||
            ov.tools !== undefined);
        return hasOv ? { agent_type: at, overrides: ov } : at;
      })
      .filter((e) => (typeof e === "string" ? e.length > 0 : e.agent_type.length > 0));
    const active = get().active;
    // Sessions in a project share a single, project-scoped agent flow: editing
    // the flow on any conversation persists to the project so all of its
    // conversations (and any new ones) stay in sync. Plain chats keep a
    // per-session chain.
    const projectId = active?.session.id === id ? active.session.project_id : null;
    if (active && active.session.id === id) {
      set({
        active: {
          ...active,
          session: { ...active.session, agent_chain: cleaned.length ? cleaned : null },
        },
      });
    }
    try {
      if (projectId) {
        await api.setProjectAgentChain(projectId, cleaned);
      } else {
        await api.setSessionAgentChain(id, cleaned);
      }
    } catch (e) {
      console.warn(e);
      await get().reloadActiveSession();
    }
  },

  addAttachments: async (files) => {
    const sid = await get().ensureActive();
    const cur = get().composer;
    const room = MAX_FILES - cur.attachments.length - cur.pendingAttachments.length;
    const uploads: Array<{ file: File; pending: PendingAttachmentDraft }> = [];
    let rejected = 0;
    let imageCount = cur.attachments.filter((a) =>
      a.mime.startsWith("image/"),
    ).length;
    let audioCount = cur.attachments.filter((a) =>
      a.mime.startsWith("audio/"),
    ).length;
    const videoModel = isVideoModel();
    for (const f of files) {
      if (uploads.length >= room) {
        rejected++;
        continue;
      }
      const mime = uploadMime(f);
      if (!ACCEPT_TYPES.includes(mime)) {
        rejected++;
        continue;
      }
      const isImage = mime.startsWith("image/");
      const isAudio = mime.startsWith("audio/");
      const invalidForMode = videoModel
        ? cur.videoMode === "text" ||
          ((cur.videoMode === "first_frame" ||
            cur.videoMode === "first_last") &&
            (!isImage ||
              imageCount >= (cur.videoMode === "first_frame" ? 1 : 2))) ||
          (cur.videoMode === "reference" &&
            ((!isImage && !isAudio) ||
              (isImage && imageCount >= 9) ||
              (isAudio && audioCount >= 3)))
        : !isImage;
      if (invalidForMode) {
        rejected++;
        continue;
      }
      if (f.size > maxBytesForMime(mime)) {
        rejected++;
        continue;
      }
      uploads.push({
        file: f,
        pending: makePendingAttachment(f.name || "image", f.size),
      });
      if (isImage) imageCount += 1;
      if (isAudio) audioCount += 1;
    }
    if (uploads.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [
            ...get().composer.pendingAttachments,
            ...uploads.map((x) => x.pending),
          ],
        },
      });
    }
    for (const { file, pending } of uploads) {
      try {
        const bytes = await fileToBytes(file);
        const d = await api.addAttachmentFromBytes(sid, file.name || "image", bytes);
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
            attachments: [...get().composer.attachments, d],
          },
        });
      } catch (e) {
        console.error(e);
        rejected++;
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
          },
        });
      }
    }
    if (rejected > 0) {
      console.warn(`${rejected} file(s) rejected`);
    }
  },

  addAttachmentsFromPaths: async (paths) => {
    const sid = await get().ensureActive();
    const cur = get().composer;
    const room = MAX_FILES - cur.attachments.length - cur.pendingAttachments.length;
    const uploads = paths.slice(0, Math.max(0, room)).map((path) => ({
      path,
      pending: makePendingAttachment(pathLabel(path), null),
    }));
    let rejected = 0;
    rejected += Math.max(0, paths.length - uploads.length);
    if (uploads.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [
            ...get().composer.pendingAttachments,
            ...uploads.map((x) => x.pending),
          ],
        },
      });
    }
    for (const { path, pending } of uploads) {
      try {
        const d = await api.addAttachmentFromPath(sid, path);
        const latest = get().composer;
        const imageCount = latest.attachments.filter((a) =>
          a.mime.startsWith("image/"),
        ).length;
        const audioCount = latest.attachments.filter((a) =>
          a.mime.startsWith("audio/"),
        ).length;
        const isImage = d.mime.startsWith("image/");
        const isAudio = d.mime.startsWith("audio/");
        const videoModel = isVideoModel();
        const invalidForMode =
          d.mime.startsWith("video/") ||
          (videoModel
            ? latest.videoMode === "text" ||
              ((latest.videoMode === "first_frame" ||
                latest.videoMode === "first_last") &&
                (!isImage ||
                  imageCount >=
                    (latest.videoMode === "first_frame" ? 1 : 2))) ||
              (latest.videoMode === "reference" &&
                ((!isImage && !isAudio) ||
                  (isImage && imageCount >= 9) ||
                  (isAudio && audioCount >= 3)))
            : !isImage);
        if (invalidForMode) {
          await api.removeAttachmentDraft(d.image_id).catch(() => {});
          rejected++;
          set({
            composer: {
              ...get().composer,
              pendingAttachments: get().composer.pendingAttachments.filter(
                (p) => p.id !== pending.id,
              ),
            },
          });
          continue;
        }
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
            attachments: [...get().composer.attachments, d],
          },
        });
      } catch (e) {
        console.error(e);
        rejected++;
        set({
          composer: {
            ...get().composer,
            pendingAttachments: get().composer.pendingAttachments.filter((p) => p.id !== pending.id),
          },
        });
      }
    }
    if (rejected > 0) {
      console.warn(`${rejected} file(s) rejected`);
    }
  },

  addAttachmentFromPath: async (path) => {
    await get().addAttachmentsFromPaths([path]);
  },

  addReferenceVideoUrl: async (url) => {
    const normalized = url.trim();
    if (!normalized || !/^(https?:\/\/|asset:\/\/)/i.test(normalized)) return;
    const current = get().composer;
    const videoCount = current.attachments.filter((a) => a.mime.startsWith("video/")).length;
    if (!isVideoModel() || current.videoMode !== "reference" || videoCount >= 3) return;
    const sid = await get().ensureActive();
    try {
      const draft = await api.addUrlAttachment(sid, normalized);
      set({
        composer: {
          ...get().composer,
          attachments: [...get().composer.attachments, draft],
        },
      });
    } catch (error) {
      console.error(error);
    }
  },

  removeAttachment: async (imageId) => {
    set({
      composer: {
        ...get().composer,
        attachments: get().composer.attachments.filter(
          (a) => a.image_id !== imageId,
        ),
      },
    });
    try {
      await api.removeAttachmentDraft(imageId);
    } catch (e) {
      console.warn(e);
    }
  },

  replaceAttachment: (oldId, draft) => {
    set({
      composer: {
        ...get().composer,
        attachments: get().composer.attachments.map((a) =>
          a.image_id === oldId ? draft : a,
        ),
      },
    });
  },

  clearComposer: () => {
    set({
      composer: {
        ...get().composer,
        prompt: "",
        mentions: [],
        attachments: [],
        pendingAttachments: [],
      },
    });
  },

  setAskUserIndex: (index) => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    if (index < 0 || index >= pending.questions.length) return;
    if (index === pending.activeIndex) return;
    const flushed = flushAskUserPrompt(pending, get().composer.prompt);
    const next: PendingAskUser = { ...flushed, activeIndex: index };
    askUserPendingBySession.set(next.sessionId, next);
    set({
      pendingAskUser: next,
      composer: {
        ...get().composer,
        // Input is custom-only — never restore option text into the editor.
        prompt: askUserCustomText(next, index),
      },
    });
  },

  setAskUserAnswer: (optionKey, optionText) => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    const q = pending.questions[pending.activeIndex];
    if (!q) return;
    const key = questionKey(q, pending.activeIndex);
    const next: PendingAskUser = {
      ...pending,
      answers: {
        ...pending.answers,
        [key]: {
          optionKey,
          optionText,
          // Selecting an option clears custom input for this question.
          custom: "",
        },
      },
    };
    askUserPendingBySession.set(next.sessionId, next);
    set({
      pendingAskUser: next,
      // Do not fill the composer — keep it empty for optional custom reply.
      composer: { ...get().composer, prompt: "" },
    });
  },

  clearAskUserAnswer: () => {
    const pending = get().pendingAskUser;
    if (!pending) return;
    const q = pending.questions[pending.activeIndex];
    if (!q) return;
    const key = questionKey(q, pending.activeIndex);
    const prev = pending.answers[key];
    const custom = prev?.custom ?? get().composer.prompt;
    const nextAnswers = { ...pending.answers };
    if (custom.trim()) {
      nextAnswers[key] = { custom };
    } else {
      delete nextAnswers[key];
    }
    const next: PendingAskUser = { ...pending, answers: nextAnswers };
    askUserPendingBySession.set(next.sessionId, next);
    set({ pendingAskUser: next });
  },

  clearPendingAskUser: () => {
    const pending = get().pendingAskUser;
    if (pending) askUserPendingBySession.delete(pending.sessionId);
    set({ pendingAskUser: null });
  },

  answerPendingAskUser: async () => {
    const c = get().composer;
    let pending = get().pendingAskUser;
    if (!pending) return;
    const activeId = get().activeId;
    if (activeId && pending.sessionId !== activeId) return;

    pending = flushAskUserPrompt(pending, c.prompt);
    askUserPendingBySession.set(pending.sessionId, pending);
    set({ pendingAskUser: pending });

    const unfinished = firstUnansweredAskUserIndex(pending);
    if (unfinished >= 0) {
      if (unfinished !== pending.activeIndex) {
        get().setAskUserIndex(unfinished);
      }
      return;
    }

    const answer = formatAskUserReply(pending).trim();
    if (!answer) return;
    const items = formatAskUserItems(pending);
    const promptId = pending.blockId;

    askUserPendingBySession.delete(pending.sessionId);
    set({
      pendingAskUser: null,
      composer: { ...get().composer, prompt: "", mentions: [] },
    });
    composerDrafts.delete(pending.sessionId);

    try {
      await api.answerAskUser(promptId, answer, items);
    } catch (e) {
      console.warn("[atelier] answer_ask_user failed", e);
    }
  },

  appendMessages: (msgs) => {
    const a = get().active;
    if (!a) return;
    set({ active: { ...a, messages: [...a.messages, ...msgs] } });
  },

  quoteMessage: async (m) => {
    const a = get().active;
    if (!a || m.session_id !== a.session.id) return;

    const quotableImages = m.images.filter((img) =>
      matchesImageRole(img.role) && img.mime.startsWith("image/"),
    );
    const room = MAX_FILES - get().composer.attachments.length - get().composer.pendingAttachments.length;
    const pending = quotableImages
      .slice(0, Math.max(0, room))
      .map((img) => makePendingAttachment(pathLabel(img.rel_path), img.bytes));
    if (pending.length) {
      set({
        composer: {
          ...get().composer,
          pendingAttachments: [...get().composer.pendingAttachments, ...pending],
        },
      });
    }

    try {
      const drafts = await api.quoteMessageAsAttachments(a.session.id, m.id);
      const cur = get().composer.attachments;
      const room = MAX_FILES - cur.length;
      const toAdd = drafts.slice(0, Math.max(0, room));
      const newAtt = [...cur, ...toAdd];

      let prompt = get().composer.prompt;
      const text = stripMediaMentionTokens(m.text || "");
      if (text) {
        const quoted = text
          .split("\n")
          .map((line) => `> ${line}`)
          .join("\n");
        const head = `${quoted}\n\n`;
        prompt = prompt.trim() ? `${head}${prompt}` : head;
      }

      set({
        composer: {
          ...get().composer,
          attachments: newAtt,
          pendingAttachments: get().composer.pendingAttachments.filter(
            (p) => !pending.some((x) => x.id === p.id),
          ),
          prompt,
        },
      });
      if (drafts.length > room) {
        console.warn("Some images were skipped (max 8 attachments)");
      }
    } catch (e) {
      console.error(e);
      set({
        composer: {
          ...get().composer,
          pendingAttachments: get().composer.pendingAttachments.filter(
            (p) => !pending.some((x) => x.id === p.id),
          ),
        },
      });
    }
  },

  deleteMessage: async (messageId) => {
    const a = get().active;
    if (!a) return;
    const sid = a.session.id;
    const idx = a.messages.findIndex((x) => x.id === messageId);
    if (idx < 0) return;
    const target = a.messages[idx];

    await abortInFlightGeneration(sid);

    const toDelete = [messageId];
    // Same branch cut as resend: removing a user turn drops everything after it.
    if (target.role === "user") {
      toDelete.push(...persistedIdsAfter(a.messages, idx));
    }

    for (const id of toDelete) {
      try {
        await api.deleteMessage(id);
      } catch (e) {
        console.warn(e);
      }
    }
    await reloadActiveSessionIfViewing(sid);
    await refreshReaderAfterFileRollback(sid);
    await get().refreshList();
  },

  resendMessage: async (messageId) => {
    const a = get().active;
    if (!a) return;
    const sid = a.session.id;
    const idx = a.messages.findIndex((x) => x.id === messageId);
    if (idx < 0) return;
    const m = a.messages[idx];
    if (m.role !== "user") return;
    const text = (m.text || "").trim();
    const videoModel = isVideoModel();
    if (!text && (!videoModel || m.images.length === 0)) return;
    if (get().busyBySession[sid]) return;
    if (generationFlights.has(sid)) {
      bumpGenerationEpoch(sid);
      await waitForGenerationIdle(sid);
    }

    const toDelete = persistedIdsAfter(a.messages, idx);

    for (const id of toDelete) {
      try {
        await api.deleteMessage(id);
      } catch (e) {
        console.warn(e);
      }
    }

    bumpGenerationEpoch(sid);
    await reloadActiveSessionIfViewing(sid);
    await refreshReaderAfterFileRollback(sid);
    const epoch = getGenerationEpoch(sid);

    const c = get().composer;
    await get().persistComposerThinking(sid);
    const inputMedia = m.images.filter((i) => i.role === "input");
    // Always use the LATEST composer/session params on resend — never the
    // parameters recorded on the original message. If the user changed anything
    // in the composer, the resend must reflect it.
    const effectiveVideoMode = videoModel
      ? coerceVideoModeForMedia(c.videoMode, inputMedia, text)
      : null;
    if (videoModel && !effectiveVideoMode) return;
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();
    const run = (async () => {
      try {
        await api.regenerateImage(
          {
            session_id: sid,
            user_message_id: messageId,
            aspect_ratio: c.aspectRatio,
            image_size: c.imageSize,
            ...(videoModel && effectiveVideoMode
              ? {
                  video_mode: effectiveVideoMode,
                  video_duration: c.videoDuration,
                  video_resolution: c.videoResolution,
                  generate_audio: c.generateAudio,
                  watermark: c.watermark,
                }
              : {}),
          },
          effectiveSessionForLog(a.session),
        );
        if (epoch !== getGenerationEpoch(sid)) return;
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } catch (e: unknown) {
        if (epoch !== getGenerationEpoch(sid)) return;
        if (isGenerationCancelled(e)) {
          await persistPartialStreamIfAny(sid);
          await reloadActiveSessionIfViewing(sid);
          await get().refreshList();
          return;
        }
        console.error(e);
        await persistPartialStreamIfAny(sid);
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } finally {
        const wasCancelled = cancellingSessions.has(sid);
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          // Only remind when the run finished in the background: not user-
          // cancelled, and the user had already left this session.
          const remindInBackground = !wasCancelled && get().activeId !== sid;
          setSessionBusy(sid, false);
          if (remindInBackground) markSessionFinished(sid);
          if (!wasCancelled) playNotifySound();
        }
      }
    })();
    await trackGenerationFlight(sid, run);
  },

  editMessage: async (messageId, text, imageIds) => {
    const trimmed = text.trim();
    if (!trimmed && (!imageIds || imageIds.length === 0)) return;
    let updated: MessageAbs | null = null;
    try {
      await api.updateMessageText(messageId, trimmed);
      if (imageIds) {
        updated = await api.updateMessageImages(messageId, imageIds);
      }
    } catch (e) {
      console.warn(e);
      return;
    }
    const a = get().active;
    if (a) {
      set({
        active: {
          ...a,
          messages: a.messages.map((m) =>
            m.id === messageId
              ? updated
                ? { ...updated, text: trimmed }
                : { ...m, text: trimmed }
              : m,
          ),
        },
      });
    }
  },

  send: async () => {
    // AskUser blocks the in-flight generation — answer it instead of starting
    // a new user turn.
    if (get().pendingAskUser) {
      await get().answerPendingAskUser();
      return;
    }

    const c = get().composer;
    const text = c.prompt.trim();
    const videoModel = isVideoModel();
    if (videoModel ? !validateVideoComposer(c, text) : !text) return;
    if (c.pendingAttachments.length > 0) return;
    const sid = await get().ensureActive();
    if (get().busyBySession[sid]) return;
    await waitForGenerationIdle(sid);
    const epoch = getGenerationEpoch(sid);

    const optimisticId = `tmp-user-${Date.now()}`;
    const optimisticUser: MessageAbs = {
      id: optimisticId,
      session_id: sid,
      role: "user",
      text,
      params: {
        aspect_ratio: c.aspectRatio,
        image_size: c.imageSize,
        ...(videoModel
          ? {
              video_mode: c.videoMode,
              video_duration: c.videoDuration,
              video_resolution: c.videoResolution,
              generate_audio: c.generateAudio,
              watermark: c.watermark,
            }
          : {}),
      },
      created_at: Date.now(),
      images: c.attachments.map((a, i) => ({
        id: a.image_id,
        role: "input",
        rel_path: a.rel_path,
        thumb_rel_path: a.thumb_rel_path,
        abs_path: a.abs_path,
        thumb_abs_path: a.thumb_abs_path,
        mime: a.mime,
        media_role: videoModel
          ? resolvedMediaRole(c.videoMode, a.mime, i)
          : a.media_role,
        source_url: a.source_url,
        width: a.width,
        height: a.height,
        bytes: a.bytes,
        ord: i,
      })),
    };

    const attachmentIds = c.attachments.map((a) => a.image_id);
    const aspectRatio = c.aspectRatio;
    const imageSize = c.imageSize;
    await get().persistComposerThinking(sid);

    updateActiveSession(sid, (active) => ({
      ...active,
      messages: [...active.messages, optimisticUser],
    }));
    set({
      composer: { ...get().composer, prompt: "", mentions: [], attachments: [], pendingAttachments: [] },
    });
    // Clear the saved draft once the message is sent.
    composerDrafts.delete(sid);
    setSessionBusy(sid, true);
    ensureGenerationStreamListener();

    const run = (async () => {
      try {
        const active = get().active;
        const sessionForLog =
          active && active.session.id === sid
            ? effectiveSessionForLog(active.session)
            : null;
        await api.generateImage(
          {
            session_id: sid,
            prompt: text,
            attachment_ids: attachmentIds,
            aspect_ratio: aspectRatio,
            image_size: imageSize,
            ...(videoModel
              ? {
                  video_mode: c.videoMode,
                  video_duration: c.videoDuration,
                  video_resolution: c.videoResolution,
                  generate_audio: c.generateAudio,
                  watermark: c.watermark,
                }
              : {}),
          },
          sessionForLog,
        );
        if (epoch !== getGenerationEpoch(sid)) return;
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } catch (e: unknown) {
        if (epoch !== getGenerationEpoch(sid)) return;
        if (isGenerationCancelled(e)) {
          await persistPartialStreamIfAny(sid);
          try {
            await reloadActiveSessionIfViewing(sid);
            await get().refreshList();
          } catch (reloadError) {
            console.warn(reloadError);
          }
          return;
        }
        console.error(e);
        await persistPartialStreamIfAny(sid);
        await reloadActiveSessionIfViewing(sid);
        await get().refreshList();
      } finally {
        const wasCancelled = cancellingSessions.has(sid);
        cancellingSessions.delete(sid);
        if (epoch === getGenerationEpoch(sid)) {
          // Only remind when the run finished in the background: not user-
          // cancelled, and the user had already left this session.
          const remindInBackground = !wasCancelled && get().activeId !== sid;
          setSessionBusy(sid, false);
          if (remindInBackground) markSessionFinished(sid);
          if (!wasCancelled) playNotifySound();
        }
      }
    })();
    await trackGenerationFlight(sid, run);
  },

  interrupt: () => {
    const sid = get().activeId;
    if (!sid || !get().busyBySession[sid]) {
      return;
    }
    if (cancellingSessions.has(sid)) {
      return;
    }
    cancellingSessions.add(sid);
    // Stop accepting stream deltas and release the send button immediately.
    setSessionBusy(sid, false);
    freezeStreamingUi(sid);
    // Drop any in-flight AskUser questionnaire for this session.
    askUserPendingBySession.delete(sid);
    if (get().pendingAskUser?.sessionId === sid) {
      set({ pendingAskUser: null });
    }
    void api.cancelGeneration(sid).catch((e) => {
      console.warn("[atelier] cancel_generation failed", e);
    });
  },
  });
});
