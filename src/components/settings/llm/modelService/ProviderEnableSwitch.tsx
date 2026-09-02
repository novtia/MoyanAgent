export function ProviderEnableSwitch({
  enabled,
  onChange,
  title,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
  title?: string;
}) {
  return (
    <button
      type="button"
      className={`settings-toggle ${enabled ? "settings-toggle--on" : ""}`}
      role="switch"
      aria-checked={enabled}
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!enabled);
      }}
    >
      <span className="settings-toggle-thumb" />
    </button>
  );
}
