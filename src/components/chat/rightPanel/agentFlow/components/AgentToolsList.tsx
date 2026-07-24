import { toolDescription } from "../toolUtils";

export function AgentToolsList({
  tools,
  selected,
  onToggle,
  disabled,
  t,
}: {
  tools: string[];
  selected: string[];
  onToggle: (tool: string) => void;
  disabled?: boolean;
  t: (key: string, opts?: { defaultValue?: string }) => string;
}) {
  return (
    <div className="agent-flow-tools agent-flow-tools--list">
      {tools.map((tool) => (
        <label key={tool} className="agent-flow-tool-row">
          <input
            type="checkbox"
            checked={selected.includes(tool)}
            disabled={disabled}
            onChange={() => onToggle(tool)}
          />
          <span className="agent-flow-tool-name">{tool}</span>
          <span className="agent-flow-tool-desc">{toolDescription(t, tool)}</span>
        </label>
      ))}
    </div>
  );
}
