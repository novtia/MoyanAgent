import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Role, RoleMeter, RoleNsfw, RoleNsfwSemen } from "../../store/roleState";

/** `nsfw.精液` volume fields in ml (not 0-100). */
const SEMEN_ML_KEYS = ["吞精", "阴道", "肛门"] as const;
type SemenMlKey = (typeof SEMEN_ML_KEYS)[number];

const NSFW_SCALAR_KEYS = ["兴奋度", "湿润度"] as const;

interface RoleStateCardProps {
  role: Role;
}

/** Clamp any model-authored number into a sane 0-100 percentage. */
function pct(value: number, max = 100): number {
  if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) return 0;
  return Math.max(0, Math.min(100, (value / max) * 100));
}

function asMeter(v: RoleMeter | number): RoleMeter {
  if (typeof v === "number") return { value: v, max: 100 };
  return { value: Number(v?.value ?? 0), max: Number(v?.max ?? 100) || 100 };
}

/** Track which scalar keys changed since the previous render so we can flash
 * just the affected rows (smooth incremental updates per the spec). */
function useChangedKeys(snapshot: Record<string, number>): Set<string> {
  const prevRef = useRef<Record<string, number>>(snapshot);
  const [changed, setChanged] = useState<Set<string>>(new Set());

  useEffect(() => {
    const prev = prevRef.current;
    const next = new Set<string>();
    for (const [k, v] of Object.entries(snapshot)) {
      if (prev[k] !== v) next.add(k);
    }
    prevRef.current = snapshot;
    if (next.size === 0) return;
    setChanged(next);
    const t = window.setTimeout(() => setChanged(new Set()), 900);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [JSON.stringify(snapshot)]);

  return changed;
}

/** Flash when a string field changes (e.g. `nsfw.精液.外表`). */
function useChangedString(value: string | null | undefined): boolean {
  const prevRef = useRef(value);
  const [changed, setChanged] = useState(false);

  useEffect(() => {
    if (prevRef.current !== value) {
      prevRef.current = value;
      setChanged(true);
      const t = window.setTimeout(() => setChanged(false), 900);
      return () => window.clearTimeout(t);
    }
  }, [value]);

  return changed;
}

function RadarChart({
  data,
  changed,
}: {
  data: Array<[string, number]>;
  changed: Set<string>;
}) {
  const size = 168;
  const cx = size / 2;
  const cy = size / 2;
  const radius = size / 2 - 26;
  const n = data.length;

  const points = useMemo(() => {
    return data.map(([, value], i) => {
      const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
      const r = (pct(value) / 100) * radius;
      return {
        x: cx + Math.cos(angle) * r,
        y: cy + Math.sin(angle) * r,
        ax: cx + Math.cos(angle) * radius,
        ay: cy + Math.sin(angle) * radius,
        lx: cx + Math.cos(angle) * (radius + 14),
        ly: cy + Math.sin(angle) * (radius + 14),
      };
    });
  }, [data, n, radius, cx, cy]);

  if (n < 3) {
    // A polygon needs at least 3 vertices; fall back to bars for 1-2 dims.
    return (
      <div className="rs-bars">
        {data.map(([k, v]) => (
          <MeterBar key={k} label={k} value={v} max={100} flash={changed.has(k)} />
        ))}
      </div>
    );
  }

  const rings = [0.25, 0.5, 0.75, 1];
  const polygon = points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");

  return (
    <svg className="rs-radar" viewBox={`0 0 ${size} ${size}`} role="img">
      {rings.map((ring) => (
        <polygon
          key={ring}
          className="rs-radar-ring"
          points={data
            .map((_, i) => {
              const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
              const r = ring * radius;
              return `${(cx + Math.cos(angle) * r).toFixed(1)},${(cy + Math.sin(angle) * r).toFixed(1)}`;
            })
            .join(" ")}
        />
      ))}
      {points.map((p, i) => (
        <line key={i} className="rs-radar-spoke" x1={cx} y1={cy} x2={p.ax} y2={p.ay} />
      ))}
      <polygon className="rs-radar-area" points={polygon} />
      {points.map((p, i) => (
        <circle
          key={i}
          className={`rs-radar-dot ${changed.has(data[i][0]) ? "is-changed" : ""}`}
          cx={p.x}
          cy={p.y}
          r={changed.has(data[i][0]) ? 4 : 2.6}
        />
      ))}
      {points.map((p, i) => (
        <text
          key={i}
          className="rs-radar-label"
          x={p.lx}
          y={p.ly}
          textAnchor={p.lx < cx - 4 ? "end" : p.lx > cx + 4 ? "start" : "middle"}
          dominantBaseline="middle"
        >
          {data[i][0]}
          <tspan className="rs-radar-value" dx="4">
            {Math.round(data[i][1])}
          </tspan>
        </text>
      ))}
    </svg>
  );
}

function MeterBar({
  label,
  value,
  max,
  flash,
}: {
  label: string;
  value: number;
  max: number;
  flash?: boolean;
}) {
  const percentage = pct(value, max);
  return (
    <div className={`rs-meter ${flash ? "is-changed" : ""}`}>
      <div className="rs-meter-head">
        <span className="rs-meter-label">{label}</span>
        <span className="rs-meter-value">
          {Math.round(value)}
          {max !== 100 ? <span className="rs-meter-max">/{Math.round(max)}</span> : null}
        </span>
      </div>
      <div className="rs-meter-track">
        <div className="rs-meter-fill" style={{ width: `${percentage}%` }} />
      </div>
    </div>
  );
}

/** Millilitre readout — raw volume, not normalised to 0-100. */
function MlGauge({
  label,
  valueMl,
  flash,
}: {
  label: string;
  valueMl: number;
  flash?: boolean;
}) {
  const display = Number.isInteger(valueMl) ? String(valueMl) : valueMl.toFixed(1);
  return (
    <div className={`rs-ml-gauge ${flash ? "is-changed" : ""}`}>
      <span className="rs-ml-label">{label}</span>
      <span className="rs-ml-value">
        {display}
        <span className="rs-ml-unit">ml</span>
      </span>
    </div>
  );
}

/** i18n label for a known nsfw / semen field key; falls back to the raw key. */
function nsfwLabel(t: (k: string) => string, key: string, semen = false): string {
  const map: Record<string, string> = semen
    ? {
        外表: "roleState.nsfwSemenExterior",
        吞精: "roleState.nsfwSemenSwallowed",
        阴道: "roleState.nsfwSemenVaginal",
        肛门: "roleState.nsfwSemenAnal",
      }
    : {
        兴奋度: "roleState.nsfwArousal",
        湿润度: "roleState.nsfwWetness",
        状态: "roleState.nsfwStatus",
        敏感点: "roleState.nsfwSensitive",
      };
  const i18nKey = map[key];
  return i18nKey ? t(i18nKey) : key;
}

/** Structured NSFW panel: arousal bars → semen subsection → status / chips. */
function NsfwPanel({ nsfw, changed }: { nsfw: RoleNsfw; changed: Set<string> }) {
  const { t } = useTranslation();

  const semen: RoleNsfwSemen | undefined =
    nsfw.精液 && typeof nsfw.精液 === "object" ? (nsfw.精液 as RoleNsfwSemen) : undefined;

  const exteriorText =
    typeof semen?.外表 === "string" && semen.外表.trim() ? semen.外表.trim() : null;

  const mlEntries = SEMEN_ML_KEYS.map((k) => [k, semen?.[k]] as const).filter(
    ([, v]) => typeof v === "number" && Number.isFinite(v),
  ) as Array<[SemenMlKey, number]>;

  const hasSemenSection = exteriorText != null || mlEntries.length > 0;
  const exteriorChanged = useChangedString(exteriorText);

  const scalarEntries = NSFW_SCALAR_KEYS.map((k) => [k, nsfw[k]] as const).filter(
    ([, v]) => typeof v === "number",
  ) as Array<[string, number]>;

  const status = typeof nsfw.状态 === "string" ? nsfw.状态 : null;
  const sensitive = Array.isArray(nsfw.敏感点) ? nsfw.敏感点 : [];

  // Any extra top-level keys the model added beyond the canonical set.
  const reserved = new Set<string>([...NSFW_SCALAR_KEYS, "状态", "敏感点", "精液"]);
  const extras = Object.entries(nsfw).filter(([k]) => !reserved.has(k));

  return (
    <div className="rs-nsfw-body">
      {scalarEntries.length > 0 && (
        <div className="rs-bars">
          {scalarEntries.map(([k, v]) => (
            <MeterBar
              key={k}
              label={nsfwLabel(t, k)}
              value={v}
              max={100}
              flash={changed.has(k)}
            />
          ))}
        </div>
      )}

      {hasSemenSection && (
        <div className="rs-nsfw-semen">
          <div className="rs-nsfw-semen-title">{t("roleState.nsfwSemenSection")}</div>
          {exteriorText && (
            <div className={`rs-kv rs-kv-exterior ${exteriorChanged ? "is-changed" : ""}`}>
              <span className="rs-kv-key">{nsfwLabel(t, "外表", true)}</span>
              <span className="rs-kv-value">{exteriorText}</span>
            </div>
          )}
          {mlEntries.length > 0 && (
            <div className="rs-ml-list">
              {mlEntries.map(([k, v]) => (
                <MlGauge
                  key={k}
                  label={nsfwLabel(t, k, true)}
                  valueMl={v}
                  flash={changed.has(`精液.${k}`)}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {status && (
        <div className="rs-kv">
          <span className="rs-kv-key">{nsfwLabel(t, "状态")}</span>
          <span className="rs-kv-value">{status}</span>
        </div>
      )}

      {sensitive.length > 0 && (
        <div className="rs-kv rs-kv-chips">
          <span className="rs-kv-key">{nsfwLabel(t, "敏感点")}</span>
          <span className="rs-chips">
            {sensitive.map((it, i) => (
              <span key={i} className="rs-chip">
                {it}
              </span>
            ))}
          </span>
        </div>
      )}

      {extras.length > 0 && (
        <FieldGroup
          data={Object.fromEntries(extras) as Record<string, unknown>}
          changed={changed}
        />
      )}
    </div>
  );
}

/** Render leftover nsfw keys: numbers→bars, arrays→chips, strings→text. */
function FieldGroup({ data, changed }: { data: Record<string, unknown>; changed: Set<string> }) {
  const entries = Object.entries(data);
  const numbers = entries.filter(([, v]) => typeof v === "number") as Array<[string, number]>;
  const arrays = entries.filter(([, v]) => Array.isArray(v)) as Array<[string, unknown[]]>;
  const texts = entries.filter(
    ([, v]) => typeof v !== "number" && !Array.isArray(v) && v != null && typeof v !== "object",
  );

  return (
    <div className="rs-fieldgroup">
      {numbers.length > 0 && (
        <div className="rs-bars">
          {numbers.map(([k, v]) => (
            <MeterBar key={k} label={k} value={v} max={100} flash={changed.has(k)} />
          ))}
        </div>
      )}
      {texts.map(([k, v]) => (
        <div key={k} className="rs-kv">
          <span className="rs-kv-key">{k}</span>
          <span className="rs-kv-value">{String(v)}</span>
        </div>
      ))}
      {arrays.map(([k, arr]) => (
        <div key={k} className="rs-kv rs-kv-chips">
          <span className="rs-kv-key">{k}</span>
          <span className="rs-chips">
            {arr.map((it, i) => (
              <span key={i} className="rs-chip">
                {String(it)}
              </span>
            ))}
          </span>
        </div>
      ))}
    </div>
  );
}

export const RoleStateCard = memo(function RoleStateCard({ role }: RoleStateCardProps) {
  const { t } = useTranslation();
  const [nsfwOpen, setNsfwOpen] = useState(false);

  const attributes = useMemo(() => {
    const a = role.attributes;
    if (!a || typeof a !== "object") return [] as Array<[string, number]>;
    return Object.entries(a)
      .filter(([, v]) => typeof v === "number")
      .map(([k, v]) => [k, v as number] as [string, number]);
  }, [role.attributes]);

  const meters = useMemo(() => {
    const m = role.meters;
    if (!m || typeof m !== "object") return [] as Array<[string, RoleMeter]>;
    return Object.entries(m).map(([k, v]) => [k, asMeter(v as RoleMeter | number)] as [string, RoleMeter]);
  }, [role.meters]);

  // Snapshot of every scalar we track, for change flashing.
  const scalarSnapshot = useMemo(() => {
    const snap: Record<string, number> = {};
    attributes.forEach(([k, v]) => (snap[`attr:${k}`] = v));
    meters.forEach(([k, m]) => (snap[`meter:${k}`] = m.value));
    if (role.nsfw) {
      for (const [k, v] of Object.entries(role.nsfw)) {
        if (typeof v === "number") snap[`nsfw:${k}`] = v;
        if (k === "精液" && v && typeof v === "object") {
          for (const [sk, sv] of Object.entries(v as Record<string, unknown>)) {
            if (typeof sv === "number") snap[`nsfw:精液.${sk}`] = sv;
          }
        }
      }
    }
    return snap;
  }, [attributes, meters, role.nsfw]);

  const changedRaw = useChangedKeys(scalarSnapshot);
  const changedAttr = useMemo(
    () => new Set([...changedRaw].filter((k) => k.startsWith("attr:")).map((k) => k.slice(5))),
    [changedRaw],
  );
  const changedMeter = useMemo(
    () => new Set([...changedRaw].filter((k) => k.startsWith("meter:")).map((k) => k.slice(6))),
    [changedRaw],
  );
  const changedNsfw = useMemo(
    () => new Set([...changedRaw].filter((k) => k.startsWith("nsfw:")).map((k) => k.slice(5))),
    [changedRaw],
  );

  const tags = Array.isArray(role.tags) ? role.tags : [];
  const hasNsfw = role.nsfw && typeof role.nsfw === "object" && Object.keys(role.nsfw).length > 0;

  return (
    <article className="rs-card">
      <header className="rs-card-head">
        <span className="rs-avatar">{role.emoji || "🎭"}</span>
        <div className="rs-card-id">
          <span className="rs-name">{role.name || role.id}</span>
          {tags.length > 0 && (
            <span className="rs-chips rs-head-chips">
              {tags.map((tg, i) => (
                <span key={i} className="rs-chip">
                  {tg}
                </span>
              ))}
            </span>
          )}
        </div>
      </header>

      {(role.mood || role.location || role.outfit) && (
        <div className="rs-meta">
          {role.location && (
            <div className="rs-kv">
              <span className="rs-kv-key">{t("roleState.location")}</span>
              <span className="rs-kv-value">{role.location}</span>
            </div>
          )}
          {role.mood && (
            <div className="rs-kv">
              <span className="rs-kv-key">{t("roleState.mood")}</span>
              <span className="rs-kv-value">{role.mood}</span>
            </div>
          )}
          {role.outfit && (
            <div className="rs-kv">
              <span className="rs-kv-key">{t("roleState.outfit")}</span>
              <span className="rs-kv-value">{role.outfit}</span>
            </div>
          )}
        </div>
      )}

      {attributes.length > 0 && (
        <div className="rs-section">
          <RadarChart data={attributes} changed={changedAttr} />
        </div>
      )}

      {meters.length > 0 && (
        <div className="rs-section rs-bars">
          {meters.map(([k, m]) => (
            <MeterBar key={k} label={k} value={m.value} max={m.max ?? 100} flash={changedMeter.has(k)} />
          ))}
        </div>
      )}

      {hasNsfw && (
        <div className={`rs-nsfw ${nsfwOpen ? "is-open" : ""}`}>
          <button
            type="button"
            className="rs-nsfw-toggle"
            onClick={() => setNsfwOpen((v) => !v)}
            aria-expanded={nsfwOpen}
          >
            <span className="rs-nsfw-badge">NSFW</span>
            <span className="rs-nsfw-title">{t("roleState.nsfwSection")}</span>
            <ChevronIcon open={nsfwOpen} />
          </button>
          {nsfwOpen && <NsfwPanel nsfw={role.nsfw as RoleNsfw} changed={changedNsfw} />}
        </div>
      )}
    </article>
  );
});

function ChevronIcon({ open }: { open: boolean }) {
  return (
    <svg
      className={`rs-chevron ${open ? "is-open" : ""}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}
