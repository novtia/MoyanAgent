import type { ImageRefAbs, MessageAbs } from "../../../types";

export type MessageTokenUsageData = NonNullable<
  NonNullable<MessageAbs["params"]>["usage"]
>;

export interface MessageListProps {
  onPreviewImage: (img: ImageRefAbs) => void;
}

export interface MessageRowProps {
  m: MessageAbs;
  onPreviewImage: (img: ImageRefAbs) => void;
  focused: boolean;
}

export interface PlateActionsProps {
  img: ImageRefAbs;
  onPreview: () => void;
  showDivider?: boolean;
}

export interface RpgOption {
  id?: string;
  label: string;
  text?: string;
}

export interface ListFilesEntry {
  name: string;
  kind: string;
  children?: ListFilesEntry[];
  /** Text files only — one line = one paragraph (matches Read/Edit numbering). */
  paragraphs?: number;
}

export interface TodoItem {
  id: number;
  content: string;
  detail?: string;
  status: "pending" | "in_progress" | "done" | "cancelled";
}
