/**
 * Toast — lightweight event-bus-driven notification system.
 *
 * Usage:
 *   toast.success("已保存");
 *   toast.error("操作失败");
 *   toast.warning("请注意");
 *   toast.info("已复制到剪贴板");
 *
 * Mount <ToastHost /> once near the root (App.tsx).
 */

import { useEffect, useState } from "react";

// ─── Types ────────────────────────────────────────────────────────────────────

export type ToastType = "success" | "error" | "warning" | "info";

export interface ToastOptions {
  /** Duration in ms before auto-dismiss. Pass 0 to disable. Default: 3500 */
  duration?: number;
  /** Additional description text rendered below the title. */
  description?: string;
}

interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
  description?: string;
  duration: number;
  /** Whether the item is animating out */
  exiting: boolean;
}

// ─── Internal state (module-level singleton) ──────────────────────────────────

let nextId = 0;
let items: ToastItem[] = [];
const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((fn) => fn());
}

function addToast(type: ToastType, message: string, opts: ToastOptions = {}): number {
  const duration = opts.duration ?? 3500;
  const id = ++nextId;
  items = [
    ...items,
    { id, type, message, description: opts.description, duration, exiting: false },
  ];
  notify();

  if (duration > 0) {
    setTimeout(() => dismissToast(id), duration);
  }
  return id;
}

function dismissToast(id: number) {
  items = items.map((t) => (t.id === id ? { ...t, exiting: true } : t));
  notify();
  // Remove from DOM after exit animation finishes
  setTimeout(() => {
    items = items.filter((t) => t.id !== id);
    notify();
  }, 320);
}

// ─── Public API ───────────────────────────────────────────────────────────────

export const toast = {
  success: (message: string, opts?: ToastOptions) => addToast("success", message, opts),
  error:   (message: string, opts?: ToastOptions) => addToast("error",   message, opts),
  warning: (message: string, opts?: ToastOptions) => addToast("warning", message, opts),
  info:    (message: string, opts?: ToastOptions) => addToast("info",    message, opts),
  dismiss: (id: number) => dismissToast(id),
};

// ─── Host component ───────────────────────────────────────────────────────────

export function ToastHost() {
  const [, setTick] = useState(0);

  useEffect(() => {
    const update = () => setTick((n) => n + 1);
    listeners.add(update);
    return () => { listeners.delete(update); };
  }, []);

  if (items.length === 0) return null;

  return (
    <div className="toast-host" aria-live="polite" aria-atomic="false">
      {items.map((item) => (
        <ToastCard key={item.id} item={item} onDismiss={() => dismissToast(item.id)} />
      ))}
    </div>
  );
}

// ─── Individual card ──────────────────────────────────────────────────────────

function ToastCard({ item, onDismiss }: { item: ToastItem; onDismiss: () => void }) {
  return (
    <div
      className={`toast toast--${item.type}${item.exiting ? " toast--exit" : ""}`}
      role="alert"
    >
      <span className="toast-icon" aria-hidden>
        <ToastIcon type={item.type} />
      </span>
      <div className="toast-content">
        <span className="toast-message">{item.message}</span>
        {item.description && (
          <span className="toast-description">{item.description}</span>
        )}
      </div>
      <button
        type="button"
        className="toast-close"
        aria-label="关闭通知"
        onClick={onDismiss}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────────

function ToastIcon({ type }: { type: ToastType }) {
  switch (type) {
    case "success":
      return (
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="10" cy="10" r="8" />
          <path d="m6.5 10 2.5 2.5 4-5" />
        </svg>
      );
    case "error":
      return (
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="10" cy="10" r="8" />
          <path d="M10 6.5v4M10 13.5h.01" />
        </svg>
      );
    case "warning":
      return (
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M9.11 3.31 2.5 15A1 1 0 0 0 3.39 16.5h13.22A1 1 0 0 0 17.5 15L10.89 3.31a1 1 0 0 0-1.78 0Z" />
          <path d="M10 8.5v3M10 13.5h.01" />
        </svg>
      );
    case "info":
      return (
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="10" cy="10" r="8" />
          <path d="M10 9v4.5M10 6.5h.01" />
        </svg>
      );
  }
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <path d="M4 4l8 8M12 4l-8 8" />
    </svg>
  );
}
