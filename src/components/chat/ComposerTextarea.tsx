import { useEffect, useRef, forwardRef } from "react";
import { useSession } from "../../store/session";

const MAX_HEIGHT_PX = 200;

export interface ComposerTextareaProps {
  onSubmit: () => void;
  disabled?: boolean;
  placeholder: string;
}

function setRefs(
  el: HTMLTextAreaElement | null,
  inner: React.MutableRefObject<HTMLTextAreaElement | null>,
  outer: React.ForwardedRef<HTMLTextAreaElement>,
) {
  inner.current = el;
  if (typeof outer === "function") outer(el);
  else if (outer) (outer as React.MutableRefObject<HTMLTextAreaElement | null>).current = el;
}

/**
 * Shared prompt field for the composer (empty state and chat dock).
 *
 * This component subscribes to `composer.prompt` directly so that typing only
 * re-renders this small leaf — not the whole heavy `Composer` tree. Keeping the
 * controlled value isolated here is what keeps keystrokes responsive even with
 * a long chat history on screen.
 */
export const ComposerTextarea = forwardRef<HTMLTextAreaElement, ComposerTextareaProps>(
  function ComposerTextarea({ onSubmit, disabled, placeholder }, forwardedRef) {
    const innerRef = useRef<HTMLTextAreaElement | null>(null);
    const prompt = useSession((s) => s.composer.prompt);
    const setPrompt = useSession((s) => s.setPrompt);

    useEffect(() => {
      const ta = innerRef.current;
      if (!ta) return;
      ta.style.height = "auto";
      ta.style.height = `${Math.min(ta.scrollHeight, MAX_HEIGHT_PX)}px`;
    }, [prompt]);

    return (
      <textarea
        ref={(el) => setRefs(el, innerRef, forwardedRef)}
        rows={2}
        className="composer-textarea"
        placeholder={placeholder}
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          // Don't submit while an IME composition is active (e.g. choosing a
          // Chinese candidate with Enter), otherwise the keystroke that confirms
          // the candidate would also send the message.
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSubmit();
          }
        }}
        disabled={disabled}
      />
    );
  },
);
