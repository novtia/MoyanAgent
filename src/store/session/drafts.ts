import type { PendingAskUser } from "../../components/chat/askUser";
import { resolveRoleStateScope } from "../roleState";
import type { ComposerDraft, ComposerState } from "./types";
import { useSession } from "./store";

/** Per-session AskUser questionnaires (survives session switches). */
export const askUserPendingBySession = new Map<string, PendingAskUser>();

export const composerDrafts = new Map<string, ComposerDraft>();

export function saveComposerDraft(sessionId: string, composer: ComposerState) {
  composerDrafts.set(sessionId, {
    prompt: composer.prompt,
    mentions: composer.mentions,
    attachments: composer.attachments,
    aspectRatio: composer.aspectRatio,
    imageSize: composer.imageSize,
    videoMode: composer.videoMode,
    videoDuration: composer.videoDuration,
    videoResolution: composer.videoResolution,
    generateAudio: composer.generateAudio,
    watermark: composer.watermark,
  });
}

export function roleStateScopeForSession(sessionId: string): string {
  const state = useSession.getState();
  const session =
    state.active?.session.id === sessionId
      ? state.active.session
      : state.sessions.find((s) => s.id === sessionId);
  if (!session) return sessionId;
  return resolveRoleStateScope(session);
}
