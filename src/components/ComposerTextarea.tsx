import { useEffect, useRef, forwardRef } from "react";

const MAX_HEIGHT_PX = 200;

export interface ComposerTextareaProps {
  value: string;
  onChange: (event: React.ChangeEvent<HTMLTextAreaElement>) => void;
  onKeyDown?: (event: React.KeyboardEvent<HTMLTextAreaElement>) => void;
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

/** Shared prompt field for the composer (empty state and chat dock). */
export const ComposerTextarea = forwardRef<HTMLTextAreaElement, ComposerTextareaProps>(
  function ComposerTextarea(
    { value, onChange, onKeyDown, disabled, placeholder },
    forwardedRef,
  ) {
    const innerRef = useRef<HTMLTextAreaElement | null>(null);

    useEffect(() => {
      const ta = innerRef.current;
      if (!ta) return;
      ta.style.height = "auto";
      ta.style.height = `${Math.min(ta.scrollHeight, MAX_HEIGHT_PX)}px`;
    }, [value]);

    return (
      <textarea
        ref={(el) => setRefs(el, innerRef, forwardedRef)}
        rows={2}
        className="composer-textarea"
        placeholder={placeholder}
        value={value}
        onChange={onChange}
        onKeyDown={onKeyDown}
        disabled={disabled}
      />
    );
  },
);
