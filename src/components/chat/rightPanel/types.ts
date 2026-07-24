import type { ImageRefAbs } from "../../../types";

export type TabKind = "empty" | "gallery" | "agent-flow" | "role-state" | "reader";

export interface PanelTab {
  id: string;
  kind: TabKind;
  /** For reader tabs: the absolute file path bound to this tab (null = file picker). */
  path?: string | null;
}

export interface RightPanelProps {
  open: boolean;
  onClose: () => void;
  onPreviewImage: (img: ImageRefAbs) => void;
}

export type PickerKind = "gallery" | "agent-flow" | "role-state" | "reader";
