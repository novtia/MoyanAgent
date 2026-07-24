import type { ReaderFileTab } from "../../../../store/reader";

export type RightView = "tree" | "search";

export interface ReaderWorkspaceProps {
  path?: string | null;
  onOpenFile: (path: string) => void;
}

export interface ReaderEditorProps {
  tab: ReaderFileTab;
  activeHunkIndex?: number;
  onActiveHunkChange?: (index: number) => void;
}

export interface ReaderFileTreeProps {
  activePath?: string | null;
  onOpenFile: (path: string) => void;
}

export interface ReaderFindBarProps {
  disabled?: boolean;
  disabledReason?: string;
}
