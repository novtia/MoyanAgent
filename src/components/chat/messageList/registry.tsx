import type { ComponentType } from "react";
import type { AssistantBlock } from "../../../types";
import { AgentToolCard } from "./AgentToolCard";
import { AskUserChip } from "./AskUserChip";
import { BashToolCard } from "./BashToolCard";
import { DeleteDocCard } from "./DeleteDocCard";
import { GrepToolCard } from "./GrepToolCard";
import { ListFilesCard } from "./ListFilesCard";
import { ReadToolCard } from "./ReadToolCard";
import { RoleStateChip } from "./RoleStateChip";
import { StreamingDocCard } from "./StreamingDocCard";
import { ToolCallBlock } from "./ToolCallBlock";
import { WebFetchToolCard } from "./WebFetchToolCard";
import { WebSearchToolCard } from "./WebSearchToolCard";

export type ToolBlock = Extract<AssistantBlock, { type: "tool_use" }>;

type ToolCardProps = { block: ToolBlock };

const TOOL_REGISTRY: Record<string, ComponentType<ToolCardProps>> = {
  CreateDoc: StreamingDocCard,
  Edit: StreamingDocCard,
  Write: StreamingDocCard,
  Read: ReadToolCard,
  Delete: DeleteDocCard,
  ListFiles: ListFilesCard,
  Bash: BashToolCard,
  Grep: GrepToolCard,
  WebSearch: WebSearchToolCard,
  WebFetch: WebFetchToolCard,
  Agent: AgentToolCard,
  RoleState: RoleStateChip,
  AskUser: AskUserChip,
};

export function resolveToolComponent(
  tool: string,
): ComponentType<ToolCardProps> {
  return TOOL_REGISTRY[tool] ?? ToolCallBlock;
}
