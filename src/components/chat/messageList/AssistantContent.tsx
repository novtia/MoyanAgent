import { useMemo } from "react";
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

export function AssistantContent({
  blocks,
  isStreaming,
  suppressText,
}: {
  blocks: AssistantBlock[];
  isStreaming: boolean;
  /**
   * When the message text was manually edited it diverges from the text held in
   * `blocks`. In that case the edited `m.text` is rendered separately, so the
   * original text blocks must be suppressed here to avoid showing stale content
   * alongside the edit.
   */
  suppressText?: boolean;
}) {
  const lastThinkingIdx = useMemo(() => {
    for (let i = blocks.length - 1; i >= 0; i--) {
      if (blocks[i].type === "thinking") return i;
    }
    return -1;
  }, [blocks]);

  // Pre-compute once so the map below can check cheaply.
  const firstTodoIdx = useMemo(
    () => blocks.findIndex((b) => b.type === "tool_use" && b.tool === "TodoList"),
    [blocks],
  );
  const toolBlocks = useMemo(
    () =>
      blocks.filter(
        (b): b is Extract<AssistantBlock, { type: "tool_use" }> =>
          b.type === "tool_use",
      ),
    [blocks],
  );

  return (
    <>
      {blocks.map((block, i) => {
        if (block.type === "thinking") {
          const trailing = i === lastThinkingIdx;
          return (
            <ThinkingBlock
              key={`thinking:${i}`}
              content={block.content}
              streaming={isStreaming && trailing}
            />
          );
        }
        if (block.type === "text") {
          if (suppressText || !block.content) return null;
          return (
            <div key={`text:${i}`} className="text">
              {block.content}
            </div>
          );
        }
        if (block.type === "agent_stage") {
          return (
            <AgentStageDivider
              key={`stage:${i}`}
              label={block.name || block.agent_type}
            />
          );
        }
        // All TodoList blocks are collapsed into one persistent view;
        // only the first occurrence renders the master card.
        if (block.tool === "TodoList") {
          if (i === firstTodoIdx) {
            return (
              <TodoMasterView
                key="todo-master"
                toolBlocks={toolBlocks}
                isStreaming={isStreaming}
              />
            );
          }
          return null;
        }
        // RoleState mutations render as a terse one-line chip; the full board
        // lives in the right-panel "role state" tab.
        if (block.tool === "RoleState") {
          return <RoleStateChip key={`role:${block.id}:${i}`} block={block} />;
        }
        // RpgChoice renders as clickable option buttons; picking one inserts
        // the option's text into the composer input box.
        if (block.tool === "RpgChoice") {
          return <RpgChoiceCard key={`rpg:${block.id}:${i}`} block={block} />;
        }
        // CreateDoc renders as a dedicated document card; clicking it opens
        // the freshly created file in the reader panel.
        if (block.tool === "CreateDoc" || block.tool === "Edit") {
          return <StreamingDocCard key={`doc:${block.id}`} block={block} />;
        }
        // Delete renders as a dedicated "removed document" card, distinct from
        // the generic tool-call block.
        if (block.tool === "Delete") {
          return <DeleteDocCard key={`del:${block.id}:${i}`} block={block} />;
        }
        if (block.tool === "Read") {
          return <ReadToolCard key={`read:${block.id}:${i}`} block={block} />;
        }
        return <ToolCallBlock key={`tool:${block.id}:${i}`} block={block} />;
      })}
    </>
  );
}
