import type { ReactNode } from "react";

const stroke = {
  fill: "none" as const,
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden {...stroke}>
      {children}
    </svg>
  );
}

export function ToolGlyph({ tool }: { tool: string }) {
  switch (tool) {
    case "Read":
      return (
        <Icon>
          <path d="M12 6.4c-1.7-1.1-3.8-1.5-6.4-.6v12c2.6-.9 4.7-.5 6.4.6" />
          <path d="M12 6.4c1.7-1.1 3.8-1.5 6.4-.6v12c-2.6-.9-4.7-.5-6.4.6" />
          <path d="M12 6.4v12" />
        </Icon>
      );
    case "Grep":
      return (
        <Icon>
          <circle cx="10.8" cy="10.8" r="5.6" />
          <path d="m15.2 15.2 3.6 3.6" />
        </Icon>
      );
    case "ListFiles":
      return (
        <Icon>
          <path d="M4 8.4V7.2A1.2 1.2 0 0 1 5.2 6h3.3l1.6 1.7h8.7A1.2 1.2 0 0 1 20 8.9v7.9a1.2 1.2 0 0 1-1.2 1.2H5.2A1.2 1.2 0 0 1 4 16.8z" />
        </Icon>
      );
    case "CreateDoc":
      return (
        <Icon>
          <path d="M7 4.2h6.1L17.6 8.8V19.8H7z" />
          <path d="M13.1 4.2v4.6h4.5" />
          <path d="M12.3 12v4.2M10.2 14.1h4.2" />
        </Icon>
      );
    case "Write":
      return (
        <Icon>
          <path d="M7 4.2h6.1L17.6 8.8V19.8H7z" />
          <path d="M13.1 4.2v4.6h4.5" />
          <path d="M9.3 13h5.4M9.3 16.2h3.4" />
        </Icon>
      );
    case "Edit":
      return (
        <Icon>
          <path d="M4.5 7.5h9" />
          <path d="M4.5 12h6" />
          <path d="M4.5 16.5h5" />
          <path d="m14.2 14.6 4.6-4.6 1.7 1.7-4.6 4.6h-1.7z" />
        </Icon>
      );
    case "Delete":
      return (
        <Icon>
          <path d="M5 8h14" />
          <path d="M9.2 8V6.4A1.2 1.2 0 0 1 10.4 5.2h3.2a1.2 1.2 0 0 1 1.2 1.2V8" />
          <path d="m7.3 8 .7 10.6h8l.7-10.6" />
          <path d="M10.3 11.4v4.2M13.7 11.4v4.2" />
        </Icon>
      );
    case "Rename":
      return (
        <Icon>
          <path d="M7 8h10" />
          <path d="m14 5.4 3 2.6-3 2.6" />
          <path d="M17 16H7" />
          <path d="m10 13.4-3 2.6 3 2.6" />
        </Icon>
      );
    case "SplitDoc":
      return (
        <Icon>
          <path d="M4.2 5.2h6.2v13.6H4.2z" />
          <path d="M13.6 5.2h6.2v13.6h-6.2z" />
        </Icon>
      );
    case "Bash":
      return (
        <Icon>
          <path d="m5.2 8 4.2 4-4.2 4" />
          <path d="M12 16.5h7" />
        </Icon>
      );
    case "WebSearch":
      return (
        <Icon>
          <circle cx="12" cy="12" r="6.6" />
          <path d="M5.4 12h13.2" />
          <path d="M12 5.4c1.7 1.8 2.5 4.2 2.5 6.6s-.8 4.8-2.5 6.6c-1.7-1.8-2.5-4.2-2.5-6.6s.8-4.8 2.5-6.6z" />
        </Icon>
      );
    case "WebFetch":
      return (
        <Icon>
          <path d="M10.5 6.2H6.6A1.6 1.6 0 0 0 5 7.8v9.6A1.6 1.6 0 0 0 6.6 19h9.6a1.6 1.6 0 0 0 1.6-1.6v-3.9" />
          <path d="M13 5h6v6" />
          <path d="M19 5l-7.4 7.4" />
        </Icon>
      );
    case "NovelAI":
      return (
        <Icon>
          <rect x="4.2" y="5.2" width="15.6" height="13.6" rx="2" />
          <circle cx="9" cy="10" r="1.25" />
          <path d="m6 16.2 3.3-2.8 2.2 1.8 2.5-2.2 4 3.2" />
        </Icon>
      );
    case "Agent":
      return (
        <Icon>
          <circle cx="12" cy="6" r="2" />
          <circle cx="6.2" cy="17.4" r="2" />
          <circle cx="17.8" cy="17.4" r="2" />
          <path d="M12 8v2.2" />
          <path d="m12 10.2-4.6 5.2" />
          <path d="m12 10.2 4.6 5.2" />
        </Icon>
      );
    case "RoleState":
      return (
        <Icon>
          <circle cx="12" cy="8.4" r="2.8" />
          <path d="M6.4 19v-.3a5.6 5.6 0 0 1 11.2 0V19" />
        </Icon>
      );
    case "ConsultRoles":
      return (
        <Icon>
          <circle cx="8" cy="8.6" r="2" />
          <circle cx="16" cy="8.6" r="2" />
          <path d="M4.4 18.6v-.2a3.4 3.4 0 0 1 6.2-1.9" />
          <path d="M13.4 16.5a3.4 3.4 0 0 1 6.2 1.9v.2" />
        </Icon>
      );
    case "AskUser":
      return (
        <Icon>
          <path d="M5.5 6.2h13A1.4 1.4 0 0 1 19.9 7.6v6a1.4 1.4 0 0 1-1.4 1.4H11l-3.2 2.5v-2.5H5.5A1.4 1.4 0 0 1 4.1 13.6v-6A1.4 1.4 0 0 1 5.5 6.2z" />
          <path d="M8.8 10.6h.01M12 10.6h.01M15.2 10.6h.01" />
        </Icon>
      );
    case "TodoList":
      return (
        <Icon>
          <path d="M9.2 7h10" />
          <path d="M9.2 12h10" />
          <path d="M9.2 17h10" />
          <path d="m4 7.2 1.2 1.2 2.1-2.2" />
          <path d="m4 12.2 1.2 1.2 2.1-2.2" />
          <circle cx="5.4" cy="17" r="0.9" fill="currentColor" stroke="none" />
        </Icon>
      );
    case "FixToolCall":
      return (
        <Icon>
          <path d="M19 12.2a7 7 0 1 1-2-4.9" />
          <path d="M19 4.6V8h-3.4" />
        </Icon>
      );
    default:
      return (
        <Icon>
          <rect x="5" y="5" width="14" height="14" rx="3" />
        </Icon>
      );
  }
}
