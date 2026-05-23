/**
 * Dialog — promise-based modal replacement for window.alert / confirm / prompt.
 *
 * Usage:
 *   await dialog.alert("操作成功");
 *   const ok = await dialog.confirm("确定要删除吗？", { type: "danger" });
 *   const name = await dialog.prompt("请输入新名称", { defaultValue: "项目" });
 *
 * Mount <DialogHost /> once near the root (App.tsx).
 */

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type DialogVariant = "default" | "warning" | "danger";
export type DialogKind = "alert" | "confirm" | "prompt";

export interface DialogOptions {
  /** Visual emphasis. Default: "default" */
  type?: DialogVariant;
  /** Header title. If omitted the dialog has no dedicated title row. */
  title?: string;
  /** Label for the primary action button. Default: "确定" */
  confirmLabel?: string;
  /** Label for the cancel button. Default: "取消" */
  cancelLabel?: string;
  /** (prompt only) Pre-filled input value. */
  defaultValue?: string;
  /** (prompt only) Placeholder text for the input. */
  placeholder?: string;
}

interface DialogState {
  id: number;
  kind: DialogKind;
  message: string;
  options: Required<DialogOptions>;
  resolve: (value: string | boolean | null) => void;
}

// ─── Internal singleton ───────────────────────────────────────────────────────

let nextDialogId = 0;
let current: DialogState | null = null;
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((fn) => fn());
}

function open(
  kind: DialogKind,
  message: string,
  opts: DialogOptions,
): Promise<string | boolean | null> {
  return new Promise((resolve) => {
    // If another dialog is open, auto-resolve it (fallback safety).
    current?.resolve(kind === "prompt" ? null : false);

    current = {
      id: ++nextDialogId,
      kind,
      message,
      options: {
        type: opts.type ?? "default",
        title: opts.title ?? "",
        confirmLabel: opts.confirmLabel ?? "确定",
        cancelLabel: opts.cancelLabel ?? "取消",
        defaultValue: opts.defaultValue ?? "",
        placeholder: opts.placeholder ?? "",
      },
      resolve: (value) => {
        current = null;
        notify();
        resolve(value);
      },
    };
    notify();
  });
}

// ─── Public API ───────────────────────────────────────────────────────────────

export const dialog = {
  /** Simple acknowledgement dialog. Resolves when the user clicks OK. */
  alert: (message: string, opts: DialogOptions = {}): Promise<void> =>
    open("alert", message, opts).then(() => undefined),

  /** Yes / No dialog. Resolves `true` on confirm, `false` on cancel / close. */
  confirm: (message: string, opts: DialogOptions = {}): Promise<boolean> =>
    open("confirm", message, opts) as Promise<boolean>,

  /** Text input dialog. Resolves with the string, or `null` on cancel / close. */
  prompt: (message: string, opts: DialogOptions = {}): Promise<string | null> =>
    open("prompt", message, opts) as Promise<string | null>,
};

// ─── Host component ───────────────────────────────────────────────────────────

export function DialogHost() {
  const [, setTick] = useState(0);

  useEffect(() => {
    const update = () => setTick((n) => n + 1);
    listeners.add(update);
    return () => { listeners.delete(update); };
  }, []);

  if (!current) return null;
  return <DialogModal state={current} />;
}

// ─── Modal rendering ──────────────────────────────────────────────────────────

function DialogModal({ state }: { state: DialogState }) {
  const { kind, message, options } = state;
  const [inputValue, setInputValue] = useState(options.defaultValue);
  const inputRef = useRef<HTMLInputElement>(null);
  const confirmBtnRef = useRef<HTMLButtonElement>(null);

  const isDanger = options.type === "danger";
  const isWarning = options.type === "warning";

  const handleConfirm = () => {
    if (kind === "prompt") {
      state.resolve(inputValue);
    } else if (kind === "confirm") {
      state.resolve(true);
    } else {
      state.resolve(true);
    }
  };

  const handleCancel = () => {
    state.resolve(kind === "prompt" ? null : false);
  };

  // Focus management
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      if (kind === "prompt" && inputRef.current) {
        inputRef.current.focus();
        inputRef.current.select();
      } else {
        confirmBtnRef.current?.focus();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [kind]);

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      handleCancel();
    }
  };

  const onBackdropClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) handleCancel();
  };

  const confirmClass = [
    "btn",
    isDanger ? "danger" : isWarning ? "warning" : "primary",
  ].join(" ");

  return (
    <div
      className="modal-backdrop dialog-backdrop"
      onMouseDown={onBackdropClick}
      onKeyDown={onKeyDown}
    >
      <div
        className={`dialog dialog--${options.type}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={options.title ? "dialog-title" : undefined}
        aria-describedby="dialog-body"
      >
        {/* Icon strip */}
        <div className={`dialog-icon-strip dialog-icon-strip--${options.type}`}>
          <DialogVariantIcon type={options.type} />
        </div>

        <div className="dialog-inner">
          {/* Title */}
          {options.title && (
            <h3 id="dialog-title" className="dialog-title">
              {options.title}
            </h3>
          )}

          {/* Body */}
          <p id="dialog-body" className="dialog-body">
            {message}
          </p>

          {/* Prompt input */}
          {kind === "prompt" && (
            <input
              ref={inputRef}
              className="field dialog-input"
              type="text"
              value={inputValue}
              placeholder={options.placeholder}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  handleConfirm();
                }
              }}
            />
          )}

          {/* Actions */}
          <div className="dialog-actions">
            {kind !== "alert" && (
              <button type="button" className="btn" onClick={handleCancel}>
                {options.cancelLabel}
              </button>
            )}
            <button
              ref={confirmBtnRef}
              type="button"
              className={confirmClass}
              onClick={handleConfirm}
            >
              {options.confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────────

function DialogVariantIcon({ type }: { type: DialogVariant }) {
  switch (type) {
    case "danger":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
          <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      );
    case "warning":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
      );
  }
}
