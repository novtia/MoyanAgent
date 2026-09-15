import type { ImageRefAbs } from "../../../types";

export type TabKind = "empty" | "gallery" | "agent-flow" | "role-state" | "reader";

/** Right pane of a reader tab: project tree or find results. */
export type ReaderRightView = "tree" | "search";

/** Per-tab find state (query, options and open flag live with the tab). */
export interface TabFindView {
  open: boolean;
  query: string;
  replaceWith: string;
  matchCase: boolean;
  scope: "file" | "all";
}

/** Per-tab back/forward history over reader file paths. */
export interface TabNavView {
  stack: string[];
  index: number;
}

/**
 * View state owned by a single tab. Nothing here is shared between tabs, so
 * two reader tabs keep independent split ratios, find queries and history.
 */
export interface TabViewState {
  ratio?: number;
  showTree?: boolean;
  rightView?: ReaderRightView;
  preview?: boolean;
  find?: TabFindView;
  history?: TabNavView;
}

export interface PanelTab {
  id: string;
  kind: TabKind;
  /** For reader tabs: the absolute file path bound to this tab (null = file picker). */
  path?: string | null;
  view?: TabViewState;
}

export interface RightPanelProps {
  open: boolean;
  onClose: () => void;
  onPreviewImage: (img: ImageRefAbs) => void;
}

export type PickerKind = "gallery" | "agent-flow" | "role-state" | "reader";
