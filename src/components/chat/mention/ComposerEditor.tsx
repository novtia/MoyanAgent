import { forwardRef, useCallback } from "react";
import { useSession } from "../../../store/session";
import { MentionEditor, type MentionEditorHandle } from "./MentionEditor";

export type ComposerEditorHandle = MentionEditorHandle;

export interface ComposerEditorProps {
  onSubmit: () => void;
  disabled?: boolean;
  placeholder: string;
}

/**
 * Composer field: the controlled {@link MentionEditor} bound to the session
 * store (`composer.prompt` plain text + `composer.mentions` paths).
 */
export const ComposerEditor = forwardRef<ComposerEditorHandle, ComposerEditorProps>(
  function ComposerEditor({ onSubmit, disabled, placeholder }, forwardedRef) {
    const prompt = useSession((s) => s.composer.prompt);
    const mentions = useSession((s) => s.composer.mentions);
    const setPrompt = useSession((s) => s.setPrompt);
    const setMentions = useSession((s) => s.setMentions);

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
      />
    );
  },
);
