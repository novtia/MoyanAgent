import { useMemo } from "react";
import type { AssistantBlock } from "../../../types";
import { highlightQuery } from "../../../utils/highlightQuery";
import { AgentStageDivider } from "./AgentStageDivider";
import { ChatMarkdown } from "./ChatMarkdown";
import { RetryNoticeRow } from "./RetryNoticeRow";
import { ThinkingBlock } from "./ThinkingBlock";
import { TodoMasterView } from "./TodoMasterView";
import { resolveToolComponent } from "./registry";
import { foldFixToolCalls, type TodoBlock } from "./utils";

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
  const displayBlocks = useMemo(() => foldFixToolCalls(blocks), [blocks]);

  const lastThinkingIdx = useMemo(() => {
    for (let i = displayBlocks.length - 1; i >= 0; i--) {
      if (displayBlocks[i].type === "thinking") return i;
    }
    return -1;
  }, [displayBlocks]);
  // Only the trailing thinking block that is still the tip of the stream
  // should show the live "thinking…" state. Once a tool_use / text block
  // follows, prior thinking collapses to the static toggle.
  const liveThinkingIdx = useMemo(() => {
    if (displayBlocks.length === 0) return -1;
    return displayBlocks[displayBlocks.length - 1].type === "thinking"
      ? lastThinkingIdx
      : -1;
  }, [displayBlocks, lastThinkingIdx]);

  const firstTodoIdx = useMemo(
    () =>
      displayBlocks.findIndex(
        (b) => b.type === "tool_use" && b.tool === "TodoList",
      ),
    [displayBlocks],
  );
  const toolBlocks = useMemo(
    () =>
      displayBlocks.filter(
        (b): b is TodoBlock => b.type === "tool_use",
      ),
    [displayBlocks],
  );

  return (
    <>
      {displayBlocks.map((block, i) => {
        if (block.type === "thinking") {
          const live = i === liveThinkingIdx;
          if (!block.content && !live) return null;
          return (
            <ThinkingBlock
              key={`thinking:${i}`}
              content={block.content}
              streaming={isStreaming && live}
              startedAt={block.started_at}
              durationMs={block.duration_ms}
              highlightQuery={query}
            />
          );
        }
        if (block.type === "text") {
          if (suppressText || !block.content) return null;
          return (
            <div key={`text:${i}`} className={query ? "text" : "text chat-md"}>
              {query ? (
                highlightQuery(block.content, query)
              ) : (
                <ChatMarkdown text={block.content} />
              )}
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
        if (block.type === "retry") {
          return (
            <RetryNoticeRow
              key={`retry:${i}`}
              attempt={block.attempt}
              maxRetries={block.max_retries}
              delayMs={block.delay_ms}
              reason={block.reason}
              active={isStreaming && i === displayBlocks.length - 1}
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
