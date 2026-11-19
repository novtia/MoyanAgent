import { useTranslation } from "react-i18next";
import type { TimelineLane, TraceTimelineSegment } from "./traceModel";
import { formatDurationMs } from "./traceModel";

interface TraceTimelineProps {
  segments: TraceTimelineSegment[];
  byDuration: boolean;
  activeRowId: string | null;
  onSelectRow: (rowId: string) => void;
}

const LANES: Array<{ key: TimelineLane; labelKey: string }> = [
  { key: "input", labelKey: "trace.lane.input" },
  { key: "model", labelKey: "trace.lane.model" },
  { key: "tool", labelKey: "trace.lane.tool" },
];

export function TraceTimeline({
  segments,
  byDuration,
  activeRowId,
  onSelectRow,
}: TraceTimelineProps) {
  const { t } = useTranslation();
  if (segments.length === 0) return null;

  const equalWidthPct = 100 / segments.length;

  return (
    <div className="trace-timeline" aria-label={t("trace.timelineLabel")}>
      {LANES.map((lane) => (
        <div key={lane.key} className="trace-timeline-lane">
          <span className="trace-timeline-lane-label">{t(lane.labelKey)}</span>
          <div className="trace-timeline-track">
            {segments.map((seg, idx) => {
              if (seg.lane !== lane.key) return null;
              const left = byDuration ? seg.leftPct : idx * equalWidthPct;
              const width = byDuration ? seg.widthPct : Math.max(equalWidthPct, 0.8);
              const dur = formatDurationMs(seg.durationMs);
              const title = dur ? `${seg.label} (${dur})` : seg.label;
              const isSelected = activeRowId === seg.rowId;

              return (
                <button
                  key={seg.rowId}
                  type="button"
                  title={title}
                  className={`trace-timeline-seg is-${seg.category}${seg.isError ? " is-error" : ""}${isSelected ? " is-selected" : ""}`}
                  style={{
                    left: `${left}%`,
                    width: `${width}%`,
                  }}
                  onClick={() => onSelectRow(seg.rowId)}
                />
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
