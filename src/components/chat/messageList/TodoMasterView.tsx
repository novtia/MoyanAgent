import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { replayTodoBlocks, type TodoBlock } from "./utils";
import { ThinkingChevronIcon, TodoStatusIcon } from "./icons";

export function TodoMasterView({
  todoBlocks,
  isStreaming,
}: {
  todoBlocks: TodoBlock[];
  isStreaming: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(true);

  const { items, busy } = useMemo(
    () => replayTodoBlocks(todoBlocks),
    [todoBlocks],
  );

  const totalDone = items.filter((it) => it.status === "done").length;
  const totalItems = items.length;
  const overallPending = busy || (isStreaming && totalItems === 0);
  const progressLabel = totalItems > 0 ? `${totalDone} / ${totalItems}` : null;

  return (
    <div
      className={`tool-call-block todo-list-block todo-master ${overallPending ? "pending" : "success"} ${open ? "is-open" : ""}`}
    >
      <button
        type="button"
        className="tool-call-summary"
        aria-expanded={open}
        onClick={() => totalItems > 0 && setOpen((v) => !v)}
        disabled={totalItems === 0}
      >
        <svg
          className="tool-call-icon"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M9 11l3 3L22 4" />
          <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
        </svg>
        <span className="tool-call-name">{t("message.todoListTitle")}</span>
        {progressLabel && (
          <span className="tool-call-args todo-progress">{progressLabel}</span>
        )}
        <span className="tool-call-spacer" aria-hidden />
        {busy && (
          <span className="tool-call-badge pending">{t("message.toolCallRunning")}</span>
        )}
        {totalItems > 0 && <ThinkingChevronIcon />}
      </button>

      {open && totalItems > 0 && (
        <ul className="todo-item-list" role="list">
          {items.map((item) => (
            <li key={item.id} className={`todo-item ${item.status}`}>
              <TodoStatusIcon status={item.status} />
              <span className="todo-item-content">{item.content}</span>
              <span className={`todo-item-badge ${item.status}`}>
                {item.status === "pending"
                  ? t("message.todoStatusPending")
                  : item.status === "in_progress"
                    ? t("message.todoStatusInProgress")
                    : item.status === "done"
                      ? t("message.todoStatusDone")
                      : t("message.todoStatusCancelled")}
              </span>
            </li>
          ))}
        </ul>
      )}

      {open && totalItems === 0 && !busy && (
        <p className="todo-empty">{t("message.todoListEmpty")}</p>
      )}
    </div>
  );
}
