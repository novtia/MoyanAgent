import type { ReactNode } from "react";

interface SegmentButtonProps {
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
}

export function SegmentButton({
  active,
  onClick,
  children,
}: SegmentButtonProps) {
  return (
    <button
      type="button"
      className={`settings-segment-btn ${active ? "active" : ""}`}
      onClick={onClick}
    >
      {children}
    </button>
  );
}
