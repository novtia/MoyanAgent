import fs from "fs";
import path from "path";

const OUT = "src/components/chat/messageList";
const lines = fs.readFileSync("src/components/chat/MessageList.tsx", "utf8").split("\n");
const L = (a, b) => lines.slice(a, b + 1).join("\n");
const exp = (code) =>
  code
    .replace(/^async function /gm, "export async function ")
    .replace(/^function /gm, "export function ");

fs.mkdirSync(OUT, { recursive: true });
const w = (file, content) => fs.writeFileSync(path.join(OUT, file), content.trimEnd() + "\n");

w("types.ts", `import type { ImageRefAbs, MessageAbs } from "../../../types";

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
}

export interface TodoItem {
  id: number;
  content: string;
  status: "pending" | "in_progress" | "done" | "cancelled";
}
`);

w("utils.ts", `import type { AssistantBlock } from "../../../types";
import type { ListFilesEntry, MessageTokenUsageData, RpgOption, TodoItem } from "./types";

export type TodoBlock = Extract<AssistantBlock, { type: "tool_use" }>;

export const tokenUsageFormatter = new Intl.NumberFormat();

${exp(L(46, 54))}

${exp(L(81, 94))}

${exp(L(222, 234))}

${exp(L(1314, 1322))}

${exp(L(920, 950))}

${exp(L(986, 1009))}

${L(537, 558).replace("interface RpgOption {", "// RpgOption -> types.ts\n").replace("function parseRpgChoiceInput", "export function parseRpgChoiceInput")}

${L(1115, 1191).replace("interface TodoItem {", "").replace("type TodoBlock = Extract<AssistantBlock, { type: \"tool_use\" }>;\n\n", "").replace("function parseRawItems", "function parseRawItems").replace("function replayTodoBlocks", "export function replayTodoBlocks")}
`);

w("icons.tsx", `import type { TodoItem } from "./types";

${exp(L(236, 275))}

${exp(L(866, 917))}

${exp(L(1193, 1230))}

${exp(L(1994, 2059))}
`);

w("MessageTokenUsage.tsx", `import { useTranslation } from "react-i18next";
import { resolveMessageTokenUsage, tokenUsageFormatter } from "./utils";
import type { MessageTokenUsageData } from "./types";

${exp(L(56, 79))}
`);

w("ThinkingBlock.tsx", `import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ThinkingChevronIcon, ThinkingIcon } from "./icons";

${exp(L(277, 345))}
`);

w("AgentStageDivider.tsx", `import { useTranslation } from "react-i18next";

${exp(L(467, 491))}
`);

w("RoleStateChip.tsx", `import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";

${exp(L(495, 527))}
`);

w("RpgChoiceCard.tsx", `import { useMemo } from "react";
import { useSession } from "../../../store/session";
import type { AssistantBlock } from "../../../types";
import { parseRpgChoiceInput } from "./utils";
import type { RpgOption } from "./types";

${exp(L(563, 600))}
`);

w("StreamingDocCard.tsx", `import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { diffChars } from "diff";
import { countChars, readerDocFromToolOutput, useReader } from "../../../store/reader";
import { InlineDiffCode } from "../../../utils/inlineDiff";
import type { AssistantBlock } from "../../../types";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

${exp(L(607, 770))}
`);

w("ReadToolCard.tsx", `import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatReadToolTitle } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

${exp(L(773, 825))}
`);

w("DeleteDocCard.tsx", `import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { ToolCallIcon } from "./icons";

${exp(L(828, 864))}
`);

w("ToolCallBlock.tsx", `import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import type { ListFilesEntry } from "./types";
import { parseListFilesOutput, safeJsonStringify, summarizeToolInput } from "./utils";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

${exp(L(952, 960))}

${exp(L(962, 978))}

${exp(L(1011, 1098))}
`);

w("TodoMasterView.tsx", `import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { replayTodoBlocks, type TodoBlock } from "./utils";
import { ThinkingChevronIcon, TodoStatusIcon } from "./icons";

${exp(L(1232, 1312))}
`);

w("AssistantContent.tsx", `import { useMemo } from "react";
import type { AssistantBlock } from "../../../types";
import { AgentStageDivider } from "./AgentStageDivider";
import { DeleteDocCard } from "./DeleteDocCard";
import { ReadToolCard } from "./ReadToolCard";
import { RoleStateChip } from "./RoleStateChip";
import { RpgChoiceCard } from "./RpgChoiceCard";
import { StreamingDocCard } from "./StreamingDocCard";
import { ThinkingBlock } from "./ThinkingBlock";
import { TodoMasterView } from "./TodoMasterView";
import { ToolCallBlock } from "./ToolCallBlock";
import type { TodoBlock } from "./utils";

${exp(L(358, 464))}
`);

w("DevelopingRow.tsx", `import { useTranslation } from "react-i18next";
import { nowStamp } from "./utils";

${exp(L(1973, 1992))}
`);

w("PlateActions.tsx", `import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { api, srcOf } from "../../../api/tauri";
import type { PlateActionsProps } from "./types";
import { CopyIcon, DownloadIcon, ZoomIcon } from "./icons";

${exp(L(1922, 1971))}
`);

w("MessageList.tsx", `import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import type { MessageListProps } from "./types";
import { DevelopingRow } from "./DevelopingRow";
import { MessageRow } from "./MessageRow";

${exp(L(112, 220))}
`);

w("MessageRow.tsx", `import {
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, srcOf } from "../../../api/tauri";
import { dialog, toast } from "../../ui";
import { useProject } from "../../../store/project";
import { useSession } from "../../../store/session";
import { ATELIER_DRAG_TYPE } from "../rightPanel/gallery";
import { READER_FILE_DRAG_TYPE } from "../../../utils/readerDrag";
import {
  MentionEditor,
  MentionText,
  isWithinProject,
  normalizeMentionPath,
  parseMentionPaths,
  type MentionEditorHandle,
} from "../mention";
import type { AssistantBlock, AttachmentDraft, ImageRefAbs } from "../../../types";
import { AssistantContent } from "./AssistantContent";
import { MessageTokenUsage } from "./MessageTokenUsage";
import { PlateActions } from "./PlateActions";
import { ThinkingBlock } from "./ThinkingBlock";
import type { MessageRowProps } from "./types";
import {
  copyText,
  fileToBytes,
  isImageFile,
  nativeFilePath,
  nowStamp,
} from "./utils";
import {
  CopyIcon,
  EditIcon,
  PlusIcon,
  QuoteIcon,
  ResendIcon,
  TrashIcon,
} from "./icons";

${L(1324, 1912)}

export const MessageRow = memo(MessageRowImpl, (prev, next) => {
  return (
    prev.m === next.m &&
    prev.focused === next.focused &&
    prev.onPreviewImage === next.onPreviewImage
  );
});
`);

w("index.ts", `export { MessageList } from "./MessageList";
export type { MessageListProps } from "./types";
`);

console.log("messageList modules generated.");
