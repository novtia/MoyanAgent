import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { replayTodoState, type TodoBlock } from "./utils";

export function TodoMasterView({
  toolBlocks,
  isStreaming,
}: {
  toolBlocks: TodoBlock[];
  isStreaming: boolean;
}) {
  const { t } = useTranslation();

  const { items, busy } = useMemo(
    () => replayTodoState(toolBlocks),
    [toolBlocks],
  );

  const totalDone = items.filter((it) => it.status === "done").length;
  const totalItems = items.length;
  const overallPending = busy || (isStreaming && totalItems === 0);
  const pct = totalItems > 0 ? Math.round((totalDone / totalItems) * 100) : 0;

  return (
    <div
      className={`todo${overallPending ? " is-pending" : ""}`}
      role="region"
      aria-label={t("message.todoListTitle")}
    >
      <div className="todo-progress">
        <span className="kv" style={{ fontWeight: 600, color: "var(--ink-soft)" }}>
          {t("message.todoListTitle")}
        </span>
        <div className="todo-bar" aria-hidden>
          <i style={{ width: `${pct}%` }} />
        </div>
        <span className="todo-count">
          {totalItems > 0 ? `${totalDone} / ${totalItems}` : "—"}
        </span>
      </div>

      {totalItems > 0 ? (
        <ul className="todo-item-list" role="list">
          {items.map((item) => {
            const doing = item.status === "in_progress";
            const done = item.status === "done";
            const cancelled = item.status === "cancelled";
            return (
              <li
                key={item.id}
                className={`todo-item${done ? " done" : ""}${doing ? " doing" : ""}${cancelled ? " cancelled" : ""}`}
              >
                <span className="todo-box" aria-hidden>
                  {done ? (
                    <svg
                      width="9"
                      height="9"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="3.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                  ) : null}
                </span>
                <div className="todo-item-main">
                  <div className="tt">{item.content}</div>
                  {item.detail && <div className="td">{item.detail}</div>}
                </div>
                {doing && (
                  <span className="todo-tag doing">
                    {t("message.todoStatusInProgress")}
                  </span>
                )}
                {cancelled && (
                  <span className="todo-tag cancelled">
                    {t("message.todoStatusCancelled")}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="todo-empty">
          {busy
            ? t("message.toolCallRunning")
            : t("message.todoListEmpty")}
        </p>
      )}
    </div>
  );
}
