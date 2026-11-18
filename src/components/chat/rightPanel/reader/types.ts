import type { ReaderFileTab } from "../../../../store/reader";
import type { RewriteMode } from "../../../../store/readerRewrite";
import type { ChunkRewriteTarget } from "./components/ChunkedEditorView";

export type RightView = "tree" | "search";

export interface ReaderWorkspaceProps {
  path?: string | null;
  onOpenFile: (path: string) => void;
}

export interface ReaderEditorProps {
  tab: ReaderFileTab;
  chunked?: boolean;
  rewriteRunning?: boolean;
  onRewriteChunk?: (mode: RewriteMode, target: ChunkRewriteTarget) => void;
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
