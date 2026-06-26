import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
} from "react";
import {
  buildMentionNodes,
  collectMentions,
  createMentionNode,
  moveCaretToEnd,
  normalizeMentionPath,
  serializeMentions,
} from "./core";

export interface MentionEditorHandle {
  focus: () => void;
  insertMention: (absPath: string, isDir?: boolean) => void;
}

export interface MentionEditorProps {
  /** Plain text value, mentions serialized as `@<path>`. */
  value: string;
  /** Mention paths present in `value`, for unambiguous chip rebuilds. */
  mentions: string[];
  onChange: (text: string, mentions: string[]) => void;
  onSubmit?: () => void;
  placeholder: string;
  disabled?: boolean;
  className?: string;
  autoFocus?: boolean;
}

/**
 * Controlled contenteditable field with inline `@file` mention chips.
 *
 * `value` (plain text, chips serialized as `@<path>`) plus `mentions` (paths,
 * for unambiguous rebuild) are the source of truth. Local typing serializes to
 * `onChange` without rewriting the DOM (keeps the caret stable); external value
 * changes rebuild the DOM from text + mentions.
 */
export const MentionEditor = forwardRef<MentionEditorHandle, MentionEditorProps>(
  function MentionEditor(
    { value, mentions, onChange, onSubmit, placeholder, disabled, className, autoFocus },
    forwardedRef,
  ) {
    const rootRef = useRef<HTMLDivElement | null>(null);
    const lastSerializedRef = useRef<string>("");
    const didInitRef = useRef(false);

    const updateEmptyState = useCallback((root: HTMLElement, text: string) => {
      root.setAttribute("data-empty", text.length === 0 ? "true" : "false");
    }, []);

    const syncFromDom = useCallback(() => {
      const root = rootRef.current;
      if (!root) return;
      const text = serializeMentions(root);
      lastSerializedRef.current = text;
      updateEmptyState(root, text);
      onChange(text, collectMentions(root));
    }, [onChange, updateEmptyState]);

    // Seed the DOM on mount (with optional autofocus), then rebuild only when
    // the external value diverges from what we last serialized.
    useEffect(() => {
      const root = rootRef.current;
      if (!root) return;
      if (!didInitRef.current) {
        didInitRef.current = true;
        root.replaceChildren(...buildMentionNodes(value, mentions));
        lastSerializedRef.current = value;
        updateEmptyState(root, value);
        if (autoFocus) {
          root.focus();
          moveCaretToEnd(root);
        }
        return;
      }
      if (value === lastSerializedRef.current) return;
      root.replaceChildren(...buildMentionNodes(value, mentions));
      lastSerializedRef.current = value;
      updateEmptyState(root, value);
      if (document.activeElement === root) moveCaretToEnd(root);
    }, [value, mentions, autoFocus, updateEmptyState]);

    useImperativeHandle(
      forwardedRef,
      () => ({
        focus: () => {
          const root = rootRef.current;
          if (!root) return;
          root.focus();
          moveCaretToEnd(root);
        },
        insertMention: (absPath: string, isDir?: boolean) => {
          const root = rootRef.current;
          if (!root) return;
          const cleanPath = normalizeMentionPath(absPath);
          root.focus();
          const sel = window.getSelection();
          let range: Range;
          if (sel && sel.rangeCount > 0 && root.contains(sel.anchorNode)) {
            range = sel.getRangeAt(0);
          } else {
            range = document.createRange();
            range.selectNodeContents(root);
            range.collapse(false);
          }
          range.deleteContents();
          const space = document.createTextNode(" ");
          const mention = createMentionNode(cleanPath, isDir);
          range.insertNode(space);
          range.insertNode(mention);
          range.setStartAfter(space);
          range.collapse(true);
          sel?.removeAllRanges();
          sel?.addRange(range);
          syncFromDom();
        },
      }),
      [syncFromDom],
    );

    return (
      <div
        ref={rootRef}
        className={className ?? "composer-editor"}
        contentEditable={!disabled}
        role="textbox"
        aria-multiline="true"
        spellCheck={false}
        data-empty="true"
        data-placeholder={placeholder}
        suppressContentEditableWarning
        onInput={syncFromDom}
        onClick={(e) => {
          const target = e.target as HTMLElement;
          const removeBtn = target.closest(".composer-mention-remove");
          if (removeBtn) {
            e.preventDefault();
            removeBtn.closest(".composer-mention")?.remove();
            syncFromDom();
          }
        }}
        onPaste={(e) => {
          e.preventDefault();
          const text = e.clipboardData.getData("text/plain");
          if (text) document.execCommand("insertText", false, text);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === "Enter" && !e.shiftKey && onSubmit) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
    );
  },
);
