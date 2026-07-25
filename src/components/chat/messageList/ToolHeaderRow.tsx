import type { ReactNode } from "react";
import { ToolGlyph } from "./toolIcons";

export type ToolStatus = "pending" | "success" | "error";

type Props = {
  tool: string;
  name?: string;
  meta?: ReactNode;
  status: ToolStatus;
  streaming?: boolean;
  open?: boolean;
  expandable?: boolean;
  bare?: boolean;
  onToggle?: () => void;
  tail?: ReactNode;
  children?: ReactNode;
  errorMessage?: string;
  errorLabel?: string;
};

/**
 * Shared header for tool rows — no border box, no left rail.
 */
export function ToolHeaderRow({
  tool,
  name,
  meta,
  status,
  streaming,
  open,
  expandable,
  bare,
  onToggle,
  tail,
  children,
  errorMessage,
  errorLabel,
}: Props) {
  const interactive = Boolean(expandable && onToggle);
  const className = [
    "doc-row",
    bare ? "doc-row--bare" : "",
    open ? "is-open" : "",
    streaming ? "is-streaming" : "",
    status === "error" ? "is-error" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const head = (
    <>
      <span className="ti">
        <ToolGlyph tool={tool} />
      </span>
      {name != null && name !== "" && (
        <span
          className="doc-row-name"
          title={typeof name === "string" ? name : undefined}
        >
          {name}
        </span>
      )}
      {meta != null && meta !== "" && (
        <span className="doc-row-meta">{meta}</span>
      )}
      {tail != null && tail !== false && (
        <span className="doc-row-tail">{tail}</span>
      )}
    </>
  );

  return (
    <div className={className}>
      {interactive ? (
        <button
          type="button"
          className="doc-row-head"
          aria-expanded={!!open}
          onClick={onToggle}
        >
          {head}
        </button>
      ) : (
        <div className="doc-row-head">{head}</div>
      )}
      {open && children != null && (
        <div className="doc-row-body">{children}</div>
      )}
      {errorMessage ? (
        <div className="tool-call-error-detail" role="alert">
          {errorLabel ? (
            <span className="tool-call-error-detail-label">{errorLabel}</span>
          ) : null}
          <span className="tool-call-error-detail-text">{errorMessage}</span>
        </div>
      ) : null}
    </div>
  );
}

/** Compact flow header used above Bash / Grep / Web* panels. */
export function FlowHead({
  tool,
  name,
  meta,
}: {
  tool: string;
  name: string;
  meta?: ReactNode;
  /** retained for call-site compatibility; unused */
  status?: ToolStatus;
}) {
  return (
    <div className="flow-head">
      <span className="ti">
        <ToolGlyph tool={tool} />
      </span>
      <span className="t">{name}</span>
      {meta != null && meta !== "" && <span className="m">{meta}</span>}
    </div>
  );
}
