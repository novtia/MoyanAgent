import { useMemo } from "react";
import type { AssistantBlock } from "../../../types";
import { highlightQuery } from "../../../utils/highlightQuery";
import { AgentStageDivider } from "./AgentStageDivider";
import { ThinkingBlock } from "./ThinkingBlock";
import { TodoMasterView } from "./TodoMasterView";
import { resolveToolComponent } from "./registry";
import type { TodoBlock } from "./utils";

export function AssistantContent({
  blocks,
  isStreaming,
  suppressText,
  highlightQuery: query,
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
  highlightQuery?: string;
}) {
  const lastThinkingIdx = useMemo(() => {
    for (let i = blocks.length - 1; i >= 0; i--) {
      if (blocks[i].type === "thinking") return i;
    }
    return -1;
  }, [blocks]);
  // Only the trailing thinking block that is still the tip of the stream
  // should show the live "thinking…" state. Once a tool_use / text block
  // follows, prior thinking collapses to the static toggle.
  const liveThinkingIdx = useMemo(() => {
    if (blocks.length === 0) return -1;
    return blocks[blocks.length - 1].type === "thinking" ? lastThinkingIdx : -1;
  }, [blocks, lastThinkingIdx]);

  const firstTodoIdx = useMemo(
    () => blocks.findIndex((b) => b.type === "tool_use" && b.tool === "TodoList"),
    [blocks],
  );
  const toolBlocks = useMemo(
    () =>
      blocks.filter(
        (b): b is TodoBlock => b.type === "tool_use",
      ),
    [blocks],
  );

  return (
    <>
      {blocks.map((block, i) => {
        if (block.type === "thinking") {
          const live = i === liveThinkingIdx;
          if (!block.content && !live) return null;
          return (
            <ThinkingBlock
              key={`thinking:${i}`}
              content={block.content}
              streaming={isStreaming && live}
              highlightQuery={query}
            />
          );
        }
        if (block.type === "text") {
          if (suppressText || !block.content) return null;
          return (
            <div key={`text:${i}`} className="text">
              {query ? highlightQuery(block.content, query) : block.content}
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
        if (block.type !== "tool_use") return null;

        // All TodoList blocks collapse into one persistent master view.
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

        const Comp = resolveToolComponent(block.tool);
        return <Comp key={`tool:${block.id}:${i}`} block={block} />;
      })}
    </>
  );
}
