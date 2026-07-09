import { forwardRef, useCallback, useMemo } from "react";
import { useSession } from "../../../store/session";
import { srcOf } from "../../../api/tauri";
import {
  MentionEditor,
  type MentionEditorHandle,
  type MentionTriggerAnchor,
} from "./MentionEditor";
import {
  mediaMentionKindFromMime,
  mediaMentionLabel,
} from "./core";

export type ComposerEditorHandle = MentionEditorHandle;

export interface ComposerEditorProps {
  onSubmit: () => void;
  disabled?: boolean;
  placeholder: string;
  onMentionTrigger?: (anchor: MentionTriggerAnchor | null) => void;
}

/**
 * Composer field: the controlled {@link MentionEditor} bound to the session
 * store (`composer.prompt` plain text + `composer.mentions` paths).
 */
export const ComposerEditor = forwardRef<ComposerEditorHandle, ComposerEditorProps>(
  function ComposerEditor(
    { onSubmit, disabled, placeholder, onMentionTrigger },
    forwardedRef,
  ) {
    const prompt = useSession((s) => s.composer.prompt);
    const mentions = useSession((s) => s.composer.mentions);
    const setPrompt = useSession((s) => s.setPrompt);
    const setMentions = useSession((s) => s.setMentions);
    const attachments = useSession((s) => s.composer.attachments);
    const mediaByPath = useMemo(() => {
      const counts = { image: 0, audio: 0, video: 0 };
      const previews: Record<string, { previewSrc?: string }> = {};
      for (const attachment of attachments) {
        const kind = mediaMentionKindFromMime(attachment.mime);
        if (!kind) continue;
        counts[kind] += 1;
        const label = mediaMentionLabel(kind, counts[kind]);
        previews[label] =
          kind === "image"
            ? {
                previewSrc: srcOf(
                  attachment.thumb_abs_path || attachment.abs_path,
                ),
              }
            : {};
      }
      return previews;
    }, [attachments]);

    const onChange = useCallback(
      (text: string, paths: string[]) => {
        setPrompt(text);
        setMentions(paths);
      },
      [setPrompt, setMentions],
    );

    return (
      <MentionEditor
        ref={forwardedRef}
        value={prompt}
        mentions={mentions}
        onChange={onChange}
        onSubmit={onSubmit}
        placeholder={placeholder}
        disabled={disabled}
        onMentionTrigger={onMentionTrigger}
        mediaByPath={mediaByPath}
      />
    );
  },
);
