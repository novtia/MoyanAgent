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
  mediaMentionDisplayLabel,
  mediaMentionIndex,
  mediaMentionKind,
  mediaMentionLabel,
  moveCaretToEnd,
  normalizeMentionPath,
  serializeMentions,
  type MediaMentionKind,
  type MentionMediaRenderData,
} from "./core";

export interface MentionEditorHandle {
  focus: () => void;
  insertMention: (absPath: string, isDir?: boolean) => void;
  insertMediaMention: (kind: MediaMentionKind) => string | null;
  replaceMentionTrigger: (
    absPath: string,
    isDir?: boolean,
    moveExisting?: boolean,
  ) => void;
  removeMention: (path: string, notify?: boolean) => boolean;
  removeAllMentions: (path: string, notify?: boolean) => boolean;
  rememberSelection: () => void;
}

export interface MentionTriggerAnchor {
  left: number;
  top: number;
  bottom: number;
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
  onRemoveMention?: (path: string, hasRemaining: boolean) => void;
  onMentionTrigger?: (anchor: MentionTriggerAnchor | null) => void;
  mediaByPath?: Record<string, MentionMediaRenderData>;
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
    {
      value,
      mentions,
      onChange,
      onSubmit,
      placeholder,
      disabled,
      className,
      autoFocus,
      onRemoveMention,
      onMentionTrigger,
      mediaByPath = {},
    },
    forwardedRef,
  ) {
    const rootRef = useRef<HTMLDivElement | null>(null);
    const lastSerializedRef = useRef<string>("");
    const lastMentionsRef = useRef<string[]>([]);
    const didInitRef = useRef(false);
    const savedRangeRef = useRef<Range | null>(null);
    const triggerRangeRef = useRef<Range | null>(null);

    const updateEmptyState = useCallback((root: HTMLElement, text: string) => {
      root.setAttribute("data-empty", text.length === 0 ? "true" : "false");
    }, []);

    const syncFromDom = useCallback(() => {
      const root = rootRef.current;
      if (!root) return;
      const text = serializeMentions(root);
      const nextMentions = collectMentions(root);
      lastSerializedRef.current = text;
      lastMentionsRef.current = nextMentions;
      updateEmptyState(root, text);
      onChange(text, nextMentions);
    }, [onChange, updateEmptyState]);

    const rememberSelection = useCallback(() => {
      const root = rootRef.current;
      const selection = window.getSelection();
      if (
        !root ||
        !selection ||
        selection.rangeCount === 0 ||
        !root.contains(selection.anchorNode)
      ) {
        return;
      }
      savedRangeRef.current = selection.getRangeAt(0).cloneRange();
    }, []);

    const updateMentionTrigger = useCallback(() => {
      const root = rootRef.current;
      const selection = window.getSelection();
      const close = () => {
        if (triggerRangeRef.current) {
          triggerRangeRef.current = null;
          onMentionTrigger?.(null);
        }
      };
      if (
        !root ||
        !selection ||
        selection.rangeCount === 0 ||
        !selection.isCollapsed ||
        !root.contains(selection.anchorNode)
      ) {
        close();
        return;
      }

      const caret = selection.getRangeAt(0);
      const container = caret.startContainer;
      const offset = caret.startOffset;
      if (
        container.nodeType !== Node.TEXT_NODE ||
        offset < 1 ||
        container.textContent?.charAt(offset - 1) !== "@"
      ) {
        close();
        return;
      }

      const triggerRange = document.createRange();
      triggerRange.setStart(container, offset - 1);
      triggerRange.setEnd(container, offset);
      triggerRangeRef.current = triggerRange;
      savedRangeRef.current = caret.cloneRange();

      const triggerRect = triggerRange.getBoundingClientRect();
      const caretRect = caret.getBoundingClientRect();
      const rect =
        triggerRect.width || triggerRect.height ? triggerRect : caretRect;
      onMentionTrigger?.({
        left: rect.right || rect.left,
        top: rect.top,
        bottom: rect.bottom,
      });
    }, [onMentionTrigger]);

    const insertMentionAtSelection = useCallback(
      (
        rawPath: string,
        isDir?: boolean,
        media?: MentionMediaRenderData,
      ): string | null => {
        const root = rootRef.current;
        if (!root) return null;
        const cleanPath = normalizeMentionPath(rawPath);
        const selection = window.getSelection();
        const liveRange =
          selection &&
          selection.rangeCount > 0 &&
          root.contains(selection.anchorNode)
            ? selection.getRangeAt(0)
            : null;
        root.focus();
        const savedRange = savedRangeRef.current;
        let range: Range;
        if (
          liveRange &&
          root.contains(liveRange.commonAncestorContainer)
        ) {
          range = liveRange;
        } else if (
          savedRange &&
          root.contains(savedRange.commonAncestorContainer)
        ) {
          range = savedRange.cloneRange();
        } else {
          range = document.createRange();
          range.selectNodeContents(root);
          range.collapse(false);
        }
        range.deleteContents();
        const space = document.createTextNode(" ");
        const mention = createMentionNode(cleanPath, isDir, media);
        range.insertNode(space);
        range.insertNode(mention);
        range.setStartAfter(space);
        range.collapse(true);
        selection?.removeAllRanges();
        selection?.addRange(range);
        savedRangeRef.current = range.cloneRange();
        syncFromDom();
        return cleanPath;
      },
      [syncFromDom],
    );

    const compactMediaMentionIndexes = useCallback(
      (root: HTMLElement, removedPath: string) => {
        const kind = mediaMentionKind(removedPath);
        const removedIndex = mediaMentionIndex(removedPath);
        if (!kind || !removedIndex) return;
        root
          .querySelectorAll<HTMLElement>(".composer-mention")
          .forEach((mention) => {
            const path = mention.dataset.path;
            if (!path || mediaMentionKind(path) !== kind) return;
            const index = mediaMentionIndex(path);
            if (!index || index <= removedIndex) return;
            const nextPath = mediaMentionLabel(kind, index - 1);
            const displayLabel = mediaMentionDisplayLabel(nextPath);
            mention.dataset.path = nextPath;
            mention.setAttribute("title", `@${displayLabel}`);
            const label = mention.querySelector<HTMLElement>(
              ".composer-mention-label",
            );
            if (label) label.textContent = displayLabel;
          });
      },
      [],
    );

    // Seed the DOM on mount (with optional autofocus), then rebuild only when
    // the external value diverges from what we last serialized.
    useEffect(() => {
      const root = rootRef.current;
      if (!root) return;
      if (!didInitRef.current) {
        didInitRef.current = true;
        root.replaceChildren(
          ...buildMentionNodes(value, mentions, mediaByPath),
        );
        savedRangeRef.current = null;
        lastMentionsRef.current = collectMentions(root);
        lastSerializedRef.current = value;
        updateEmptyState(root, value);
        if (autoFocus) {
          root.focus();
          moveCaretToEnd(root);
        }
        return;
      }
      if (value === lastSerializedRef.current) return;
      root.replaceChildren(
        ...buildMentionNodes(value, mentions, mediaByPath),
      );
      savedRangeRef.current = null;
      lastMentionsRef.current = collectMentions(root);
      lastSerializedRef.current = value;
      updateEmptyState(root, value);
      if (document.activeElement === root) moveCaretToEnd(root);
    }, [value, mentions, mediaByPath, autoFocus, updateEmptyState]);

    useImperativeHandle(
      forwardedRef,
      () => ({
        focus: () => {
          const root = rootRef.current;
          if (!root) return;
          root.focus();
          moveCaretToEnd(root);
          rememberSelection();
        },
        insertMention: (absPath: string, isDir?: boolean) => {
          const root = rootRef.current;
          if (!root) return;
          const cleanPath = normalizeMentionPath(absPath);
          insertMentionAtSelection(
            cleanPath,
            isDir,
            mediaByPath[cleanPath],
          );
        },
        insertMediaMention: (kind: MediaMentionKind) => {
          const root = rootRef.current;
          if (!root) return null;
          const maxIndex = collectMentions(root).reduce((max, path) => {
            if (mediaMentionKind(path) !== kind) return max;
            return Math.max(max, mediaMentionIndex(path) ?? 0);
          }, 0);
          const label = mediaMentionLabel(kind, maxIndex + 1);
          return insertMentionAtSelection(
            label,
            undefined,
            mediaByPath[label],
          );
        },
        replaceMentionTrigger: (
          absPath: string,
          isDir?: boolean,
          moveExisting = false,
        ) => {
          const root = rootRef.current;
          const cleanPath = normalizeMentionPath(absPath);
          const triggerRange = triggerRangeRef.current;
          if (
            !root ||
            !triggerRange ||
            !root.contains(triggerRange.commonAncestorContainer)
          ) {
            insertMentionAtSelection(
              cleanPath,
              isDir,
              mediaByPath[cleanPath],
            );
            onMentionTrigger?.(null);
            return;
          }

          if (moveExisting) {
            const existing = Array.from(
              root.querySelectorAll<HTMLElement>(".composer-mention"),
            ).find((item) => item.dataset.path === cleanPath);
            if (existing) {
              const trailing = existing.nextSibling;
              existing.remove();
              if (
                trailing?.nodeType === Node.TEXT_NODE &&
                trailing.textContent?.startsWith(" ")
              ) {
                trailing.textContent = trailing.textContent.slice(1);
                if (!trailing.textContent) trailing.remove();
              }
            }
          }

          triggerRange.deleteContents();
          triggerRange.collapse(true);
          root.focus();
          const selection = window.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(triggerRange);
          savedRangeRef.current = triggerRange.cloneRange();
          triggerRangeRef.current = null;
          insertMentionAtSelection(
            cleanPath,
            isDir,
            mediaByPath[cleanPath],
          );
          onMentionTrigger?.(null);
        },
        removeMention: (path: string, notify = true) => {
          const root = rootRef.current;
          if (!root) return false;
          const mention = Array.from(
            root.querySelectorAll<HTMLElement>(".composer-mention"),
          ).find((item) => item.dataset.path === path);
          if (!mention) return false;
          mention.remove();
          const hasRemaining = Array.from(
            root.querySelectorAll<HTMLElement>(".composer-mention"),
          ).some((item) => item.dataset.path === path);
          syncFromDom();
          if (notify) onRemoveMention?.(path, hasRemaining);
          rememberSelection();
          return true;
        },
        removeAllMentions: (path: string, notify = true) => {
          const root = rootRef.current;
          if (!root) return false;
          const matching = Array.from(
            root.querySelectorAll<HTMLElement>(".composer-mention"),
          ).filter((item) => item.dataset.path === path);
          for (const mention of matching) mention.remove();
          compactMediaMentionIndexes(root, path);
          syncFromDom();
          if (notify) onRemoveMention?.(path, false);
          rememberSelection();
          return matching.length > 0;
        },
        rememberSelection,
      }),
      [
        insertMentionAtSelection,
        compactMediaMentionIndexes,
        mediaByPath,
        onMentionTrigger,
        onRemoveMention,
        rememberSelection,
        syncFromDom,
      ],
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
        onInput={() => {
          const root = rootRef.current;
          const currentMentions = root ? collectMentions(root) : [];
          const removed = Array.from(
            new Set(
              lastMentionsRef.current.filter(
                (path) => !currentMentions.includes(path),
              ),
            ),
          );
          syncFromDom();
          for (const path of removed) onRemoveMention?.(path, false);
          rememberSelection();
          updateMentionTrigger();
        }}
        onBlur={rememberSelection}
        onKeyUp={() => {
          rememberSelection();
          updateMentionTrigger();
        }}
        onMouseUp={() => {
          rememberSelection();
          updateMentionTrigger();
        }}
        onClick={(e) => {
          const target = e.target as HTMLElement;
          const removeBtn = target.closest(".composer-mention-remove");
          if (removeBtn) {
            e.preventDefault();
            const mention = removeBtn.closest<HTMLElement>(".composer-mention");
            const path = mention?.dataset.path;
            mention?.remove();
            const hasRemaining =
              !!path &&
              Array.from(
                rootRef.current!.querySelectorAll<HTMLElement>(
                  ".composer-mention",
                ),
              ).some((item) => item.dataset.path === path);
            syncFromDom();
            if (path) onRemoveMention?.(path, hasRemaining);
            rememberSelection();
          }
        }}
        onPaste={(e) => {
          e.preventDefault();
          const text = e.clipboardData.getData("text/plain");
          if (text) document.execCommand("insertText", false, text);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === "Escape" && triggerRangeRef.current) {
            e.preventDefault();
            e.stopPropagation();
            triggerRangeRef.current = null;
            onMentionTrigger?.(null);
            return;
          }
          if (e.key === "Enter" && !e.shiftKey && onSubmit) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
    );
  },
);
