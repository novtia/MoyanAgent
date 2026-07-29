import { memo, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { api } from "../../../../api/tauri";
import { dialog } from "../../../ui";
import { useFileExplorer } from "../../../../store/fileExplorer";
import type { Role, RoleGender, RoleMeter, RoleNsfw } from "../../../../store/roleState";
import {
  SEMEN_ML_KEYS,
  nsfwScalars,
  nsfwSensitiveSpots,
  nsfwStatus,
  resolveAppearance,
  resolveGender,
  resolveSemen,
  semenMl,
  semenText,
  useRoleState,
} from "../../../../store/roleState";
import { RoleStateEditModal } from "./RoleStateEditModal";
import { ChevronIcon, LocIcon, MoodIcon, OutfitIcon, StampStarIcon } from "./icons";
import { useChangedKeys, useChangedString } from "./hooks/useChangeFlash";
import { genderLabel, nsfwLabel } from "./utils/labels";
import { asMeter, pct } from "./utils/meters";
import type { RadarDatum, RoleStateCardProps } from "./types";

function defaultMemoryRel(roleId: string): string {
  return `.moyan/trpg-memory/${roleId}.md`;
}

function joinProjectPath(root: string, rel: string): string {
  const cleaned = rel.replace(/\\/g, "/").replace(/^\/+/, "");
  const base = root.replace(/[\\/]+$/, "");
  const sep = root.includes("\\") ? "\\" : "/";
  return `${base}${sep}${cleaned.split("/").join(sep)}`;
}

function portraitGlyph(nameOrId: string): string {
  const s = nameOrId.trim();
  if (!s) return "?";
  const cjk = s.match(/[\u4e00-\u9fff]/);
  if (cjk) return cjk[0];
  const letter = s.match(/[A-Za-z0-9]/);
  if (letter) return letter[0].toUpperCase();
  return s[0];
}

function fileNo(id: string): string {
  const raw = id.trim() || "unknown";
  const clipped = raw.length > 18 ? `${raw.slice(0, 16)}...` : raw;
  return `MY-${clipped}`;
}

function GaugeTicks() {
  return (
    <>
      <span className="tick" style={{ left: "25%" }} />
      <span className="tick" style={{ left: "50%" }} />
      <span className="tick" style={{ left: "75%" }} />
    </>
  );
}

function RadarChart({
  data,
  changed,
}: {
  data: Array<RadarDatum>;
  changed: Set<string>;
}) {
  const size = 200;
  const cx = size / 2;
  const cy = size / 2;
  const radius = size / 2 - 36;
  const n = data.length;

  const points = useMemo(() => {
    return data.map(([, value, max], i) => {
      const angle = (Math.PI * 2 * i) / n - Math.PI / 2;
      const r = (pct(value, max ?? 100) / 100) * radius;
      return {
        x: cx + Math.cos(angle) * r,
        y: cy + Math.sin(angle) * r,
        ax: cx + Math.cos(angle) * radius,
        ay: cy + Math.sin(angle) * radius,
        lx: cx + Math.cos(angle) * (radius + 16),
        ly: cy + Math.sin(angle) * (radius + 16),
      };
    });
  }, [data, n, radius, cx, cy]);

  if (n < 3) {
    return (
      <div className="arc-gauges">
        {data.map(([k, v, max]) => (
          <MeterBar key={k} label={k} value={v} max={max ?? 100} flash={changed.has(k)} />
        ))}
      </div>
    );
  }

  const rings = [0.33, 0.66, 1];
  const polygon = points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");
  const areaChanged = data.some(([k]) => changed.has(k));

  return (
    <svg className="arc-radar" viewBox={`0 0 ${size} ${size}`} role="img">
      {rings.map((ring, idx) => (
        <polygon
          key={ring}
          className={`arc-radar-ring ${idx === rings.length - 1 ? "is-outer" : ""}`}
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
        <line key={i} className="arc-radar-spoke" x1={cx} y1={cy} x2={p.ax} y2={p.ay} />
      ))}
      <polygon className={`arc-radar-area ${areaChanged ? "is-changed" : ""}`} points={polygon} />
      {points.map((p, i) => (
        <circle key={`d-${i}`} className="arc-radar-dot" cx={p.x} cy={p.y} r={2.4} />
      ))}
      {points.map((p, i) => (
        <text
          key={i}
          className="arc-radar-label"
          x={p.lx}
          y={p.ly}
          textAnchor={p.lx < cx - 4 ? "end" : p.lx > cx + 4 ? "start" : "middle"}
          dominantBaseline="middle"
        >
          {data[i][0]}
          <tspan className="arc-radar-value" dx="4">
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
    <div className={`arc-gauge ${flash ? "is-changed" : ""}`}>
      <div className="arc-gauge-head">
        <span className="arc-gauge-label">{label}</span>
        <span className="arc-gauge-value">
          {Math.round(value)}
          {max !== 100 ? <span className="arc-gauge-max">/{Math.round(max)}</span> : null}
        </span>
      </div>
      <div className="arc-gauge-track">
        <GaugeTicks />
        <div className="arc-gauge-fill" style={{ width: `${percentage}%` }} />
      </div>
    </div>
  );
}

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
    <div className={`arc-ml ${flash ? "is-changed" : ""}`}>
      <span className="arc-ml-label">{label}</span>
      <span className="arc-ml-value">
        {display}
        <span className="arc-ml-unit">ml</span>
      </span>
    </div>
  );
}

function RegistryRow({
  icon,
  label,
  value,
  emptyLabel,
}: {
  icon: ReactNode;
  label: string;
  value?: string | null;
  emptyLabel: string;
}) {
  const filled = Boolean(value && String(value).trim());
  return (
    <div className="arc-row">
      <span className="arc-row-key">
        <span className="ti">{icon}</span>
        {label}
      </span>
      <span className={`arc-row-val ${filled ? "" : "is-empty"}`}>
        {filled ? value : emptyLabel}
      </span>
    </div>
  );
}

function NsfwPanel({
  nsfw,
  gender,
  changed,
}: {
  nsfw: RoleNsfw;
  gender?: RoleGender;
  changed: Set<string>;
}) {
  const { t } = useTranslation();

  const semen = resolveSemen(nsfw);
  const isMale = gender === "male";
  const isFemale = gender === "female";
  const unknownGender = gender == null;

  const textureText = isMale || unknownGender ? semenText(semen, "texture") : null;
  const exteriorText = isFemale || unknownGender ? semenText(semen, "exterior") : null;

  const mlEntries =
    isFemale || unknownGender
      ? (SEMEN_ML_KEYS.map((k) => [k, semenMl(semen, k)] as const).filter(
          ([, v]) => typeof v === "number",
        ) as Array<[(typeof SEMEN_ML_KEYS)[number], number]>)
      : [];

  const hasSemenSection =
    textureText != null || exteriorText != null || mlEntries.length > 0;
  const textureChanged = useChangedString(textureText);
  const exteriorChanged = useChangedString(exteriorText);

  const scalarEntries = nsfwScalars(nsfw);
  const status = nsfwStatus(nsfw);
  const sensitive = nsfwSensitiveSpots(nsfw);

  const reserved = new Set<string>([
    "arousal",
    "wetness",
    "status",
    "sensitive_spots",
    "semen",
    "???",
    "???",
    "??",
    "???",
    "??",
  ]);
  const extras = Object.entries(nsfw).filter(([k]) => !reserved.has(k));

  return (
    <div className="arc-sealed-body">
      {scalarEntries.length > 0 && (
        <div className="arc-gauges">
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
        <>
          {textureText && (
            <div className={`arc-kv arc-kv-exterior ${textureChanged ? "is-changed" : ""}`}>
              <span className="arc-kv-key">{nsfwLabel(t, "texture", true)}</span>
              <span className="arc-kv-value">{textureText}</span>
            </div>
          )}
          {exteriorText && (
            <div className={`arc-kv arc-kv-exterior ${exteriorChanged ? "is-changed" : ""}`}>
              <span className="arc-kv-key">{nsfwLabel(t, "exterior", true)}</span>
              <span className="arc-kv-value">{exteriorText}</span>
            </div>
          )}
          {mlEntries.length > 0 && (
            <div className="arc-ml-list">
              {mlEntries.map(([k, v]) => (
                <MlGauge
                  key={k}
                  label={nsfwLabel(t, k, true)}
                  valueMl={v}
                  flash={changed.has(`semen.${k}`) || changed.has(`??.${k}`)}
                />
              ))}
            </div>
          )}
        </>
      )}

      {status && (
        <div className="arc-sealed-note">
          <span className="k">{nsfwLabel(t, "status")}</span>
          <span>{status}</span>
        </div>
      )}

      {sensitive.length > 0 && (
        <div className="arc-sealed-note">
          <span className="k">{nsfwLabel(t, "sensitive_spots")}</span>
          <span className="arc-sealed-chips">
            {sensitive.map((it, i) => (
              <span key={i} className="arc-sealed-chip">
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

function FieldGroup({ data, changed }: { data: Record<string, unknown>; changed: Set<string> }) {
  const entries = Object.entries(data);
  const numbers = entries.filter(([, v]) => typeof v === "number") as Array<[string, number]>;
  const arrays = entries.filter(([, v]) => Array.isArray(v)) as Array<[string, unknown[]]>;
  const texts = entries.filter(
    ([, v]) => typeof v !== "number" && !Array.isArray(v) && v != null && typeof v !== "object",
  );

  return (
    <div className="arc-fieldgroup">
      {numbers.length > 0 && (
        <div className="arc-gauges">
          {numbers.map(([k, v]) => (
            <MeterBar key={k} label={k} value={v} max={100} flash={changed.has(k)} />
          ))}
        </div>
      )}
      {texts.map(([k, v]) => (
        <div key={k} className="arc-sealed-note">
          <span className="k">{k}</span>
          <span>{String(v)}</span>
        </div>
      ))}
      {arrays.map(([k, arr]) => (
        <div key={k} className="arc-sealed-note">
          <span className="k">{k}</span>
          <span className="arc-sealed-chips">
            {arr.map((it, i) => (
              <span key={i} className="arc-sealed-chip">
                {String(it)}
              </span>
            ))}
          </span>
        </div>
      ))}
    </div>
  );
}

export const RoleStateCard = memo(function RoleStateCard({
  role,
  sessionId,
  scopeId,
  isDragging,
  onCardPointerDown,
}: RoleStateCardProps) {
  const { t } = useTranslation();
  const [nsfwOpen, setNsfwOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const deleteRole = useRoleState((s) => s.deleteRole);
  const projectRoot = useFileExplorer((s) => s.projectRoot);

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

  const vitals = attributes.slice(0, 3);
  const leftoverAttrs = attributes.slice(3);

  const scalarSnapshot = useMemo(() => {
    const snap: Record<string, number> = {};
    attributes.forEach(([k, v]) => (snap[`attr:${k}`] = v));
    meters.forEach(([k, m]) => (snap[`meter:${k}`] = m.value));
    if (role.nsfw) {
      for (const [k, v] of nsfwScalars(role.nsfw)) {
        snap[`nsfw:${k}`] = v;
      }
      const semen = resolveSemen(role.nsfw);
      if (semen) {
        for (const key of SEMEN_ML_KEYS) {
          const ml = semenMl(semen, key);
          if (typeof ml === "number") snap[`nsfw:semen.${key}`] = ml;
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

  const radarData = useMemo<Array<RadarDatum>>(
    () => leftoverAttrs.map(([k, v]) => [k, v, 100] as RadarDatum),
    [leftoverAttrs],
  );

  const tags = Array.isArray(role.tags) ? role.tags : [];
  const gender = resolveGender(role);
  const appearance = resolveAppearance(role);
  const appearanceChanged = useChangedString(appearance);
  const hasNsfw = role.nsfw && typeof role.nsfw === "object" && Object.keys(role.nsfw).length > 0;
  const displayName = role.name || role.id;
  const hasGauges = meters.length > 0 || (leftoverAttrs.length > 0 && leftoverAttrs.length < 3);

  const onDelete = async () => {
    const label = role.name || role.id;
    const ok = await dialog.confirm(t("roleState.deleteConfirm", { name: label }), {
      type: "danger",
      confirmLabel: t("roleState.delete"),
    });
    if (!ok) return;
    try {
      await deleteRole(sessionId, scopeId, role.id);
    } catch (e) {
      console.warn("[roleState] delete failed", e);
    }
  };

  const controlMode = role.control === "user" ? "user" : "ai";
  const memoryRel =
    (typeof role.memory_path === "string" && role.memory_path.trim()) ||
    defaultMemoryRel(role.id);
  const hasTrpgMeta = Boolean(
    role.persona?.trim() ||
      role.goals?.trim() ||
      role.speech_style?.trim() ||
      role.control ||
      role.memory_path ||
      (typeof role.model === "string" && role.model.trim()),
  );

  const onOpenMemory = async () => {
    if (!projectRoot) {
      await dialog.alert(t("roleState.memoryNeedProject"));
      return;
    }
    const abs = joinProjectPath(projectRoot, memoryRel);
    try {
      try {
        await api.readProjectFile(sessionId, abs);
      } catch {
        await api.writeProjectFile(
          sessionId,
          abs,
          `# ${displayName} — private memory\n\n`,
        );
      }
      await api.openPath(abs);
    } catch (e) {
      console.warn("[roleState] open memory failed", e);
      await dialog.alert(t("roleState.memoryOpenFailed"));
    }
  };

  return (
    <article
      className={`arc ${isDragging ? "is-dragging" : ""}`}
      title={onCardPointerDown ? t("roleState.dragHint") : undefined}
      onPointerDown={onCardPointerDown}
    >
      <div className="arc-binding" aria-hidden>
        <span className="arc-hole" />
        <span className="arc-hole" />
      </div>

      <div className="arc-filemeta">
        <span className="arc-class">{t("roleState.classification")}</span>
        <span className="arc-fileno" title={fileNo(role.id)}>
          {t("roleState.fileNo", { id: fileNo(role.id) })}
        </span>
        <span className="arc-barcode" aria-hidden />
      </div>

      <header className="arc-cover">
        <div className="arc-portrait" aria-hidden>
          {portraitGlyph(displayName)}
          <span className="pc tl" />
          <span className="pc tr" />
          <span className="pc bl" />
          <span className="pc br" />
        </div>
        <div className="arc-cover-id">
          <div className="arc-name">
            {displayName}
            {gender && (
              <span className="arc-gender" title={genderLabel(t, gender)}>
                {genderLabel(t, gender)}
                {gender === "female" ? " / F" : " / M"}
              </span>
            )}
            <span
              className={`arc-control is-${controlMode}`}
              title={t("roleState.control")}
            >
              {controlMode === "user"
                ? t("roleState.controlUser")
                : t("roleState.controlAi")}
            </span>
          </div>
          <span className="arc-alias">{role.id}</span>
          {tags.length > 0 && (
            <div className="arc-tags">
              {tags.map((tg, i) => (
                <span key={i} className="arc-tag">
                  {tg}
                </span>
              ))}
            </div>
          )}
        </div>
        <div className="arc-actions">
          <button type="button" className="arc-act" onClick={() => setEditing(true)}>
            {t("roleState.edit")}
          </button>
          {projectRoot ? (
            <button type="button" className="arc-act" onClick={() => void onOpenMemory()}>
              {t("roleState.openMemory")}
            </button>
          ) : null}
          <button type="button" className="arc-act is-danger" onClick={() => void onDelete()}>
            {t("roleState.delete")}
          </button>
        </div>
        <span className="arc-stamp" aria-hidden>
          <StampStarIcon />
          <span>{t("roleState.stampLine1")}</span>
          <span className="st-t">{t("roleState.stampLine2")}</span>
        </span>
      </header>

      <div className="arc-registry">
        <RegistryRow
          icon={<LocIcon />}
          label={t("roleState.location")}
          value={role.location}
          emptyLabel={t("roleState.unset")}
        />
        <RegistryRow
          icon={<MoodIcon />}
          label={t("roleState.mood")}
          value={role.mood}
          emptyLabel={t("roleState.unset")}
        />
        <RegistryRow
          icon={<OutfitIcon />}
          label={t("roleState.outfit")}
          value={role.outfit}
          emptyLabel={t("roleState.unset")}
        />
      </div>

      {hasTrpgMeta && (
        <div className="arc-trpg">
          <div className="arc-field-label">
            <span className="no">TR</span>
            {t("roleState.sectionTrpg")}
          </div>
          {role.persona?.trim() ? (
            <div className="arc-trpg-line">
              <span className="k">{t("roleState.persona")}</span>
              <span>{role.persona}</span>
            </div>
          ) : null}
          {role.goals?.trim() ? (
            <div className="arc-trpg-line">
              <span className="k">{t("roleState.goals")}</span>
              <span>{role.goals}</span>
            </div>
          ) : null}
          {role.speech_style?.trim() ? (
            <div className="arc-trpg-line">
              <span className="k">{t("roleState.speechStyle")}</span>
              <span>{role.speech_style}</span>
            </div>
          ) : null}
          {typeof role.model === "string" && role.model.trim() ? (
            <div className="arc-trpg-line">
              <span className="k">{t("roleState.roleModel")}</span>
              <span className="arc-trpg-path" title={role.model}>
                {role.model}
              </span>
            </div>
          ) : null}
          <div className="arc-trpg-line">
            <span className="k">{t("roleState.memoryPath")}</span>
            <span className="arc-trpg-path" title={memoryRel}>
              {memoryRel}
            </span>
          </div>
        </div>
      )}

      {appearance && (
        <div className={`arc-appearance ${appearanceChanged ? "is-changed" : ""}`}>
          <div className="arc-field-label">
            <span className="no">01</span>
            {t("roleState.sectionAppearance")}
          </div>
          <p>{appearance}</p>
        </div>
      )}

      {vitals.length > 0 && (
        <div className={`arc-vitals ${vitals.length < 3 ? "is-sparse" : ""}`}>
          {vitals.map(([k, v]) => (
            <div key={k} className={`arc-vital ${changedAttr.has(k) ? "is-changed" : ""}`}>
              <div className="arc-vital-label">{k}</div>
              <div className="arc-vital-value">{Math.round(v)}</div>
              <div className="arc-vital-bar">
                <i style={{ width: `${pct(v, 100)}%` }} />
              </div>
            </div>
          ))}
        </div>
      )}

      {leftoverAttrs.length >= 3 && (
        <div className="arc-radar-sec">
          <div className="arc-field-label">
            <span className="no">02</span>
            {t("roleState.sectionRadar")}
          </div>
          <RadarChart data={radarData} changed={changedAttr} />
        </div>
      )}

      {hasGauges && (
        <>
          <div className="arc-sec-pad">
            <div className="arc-field-label">
              <span className="no">03</span>
              {t("roleState.sectionStatus")}
            </div>
          </div>
          <div className="arc-gauges">
            {leftoverAttrs.length > 0 &&
              leftoverAttrs.length < 3 &&
              leftoverAttrs.map(([k, v]) => (
                <MeterBar key={k} label={k} value={v} max={100} flash={changedAttr.has(k)} />
              ))}
            {meters.map(([k, m]) => (
              <MeterBar
                key={k}
                label={k}
                value={m.value}
                max={m.max ?? 100}
                flash={changedMeter.has(k)}
              />
            ))}
          </div>
        </>
      )}

      {hasNsfw && (
        <div className={`arc-sealed ${nsfwOpen ? "is-open" : ""}`}>
          <button
            type="button"
            className="arc-sealed-toggle"
            onClick={() => setNsfwOpen((v) => !v)}
            aria-expanded={nsfwOpen}
          >
            <span className="arc-wax">{t("roleState.waxSeal")}</span>
            <span className="arc-sealed-title">{t("roleState.sealedSection")}</span>
            <span className="arc-sealed-hint">{nsfwOpen ? "OPENED" : "SEALED"}</span>
            <ChevronIcon open={nsfwOpen} />
          </button>
          {nsfwOpen && (
            <NsfwPanel nsfw={role.nsfw as RoleNsfw} gender={gender} changed={changedNsfw} />
          )}
        </div>
      )}

      <div className="arc-foot">
        <span className="pg">P.1/1</span>
      </div>

      {editing && (
        <RoleStateEditModal
          role={role}
          sessionId={sessionId}
          scopeId={scopeId}
          onClose={() => setEditing(false)}
        />
      )}
    </article>
  );
});
