import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Role, RoleGender, RoleMeter, RoleNsfw } from "../../../../store/roleState";
import {
  SEMEN_ML_KEYS,
  nsfwSensitiveSpots,
  nsfwStatus,
  resolveAppearance,
  resolveGender,
  resolveSemen,
  semenMl,
  semenText,
  useRoleState,
} from "../../../../store/roleState";
import type { AttrRow, MeterRow, RoleStateEditModalProps } from "./types";
import {
  clamp01to100,
  parseAttrs,
  parseMeters,
  parseTags,
  spotsToText,
  textToSpots,
} from "./utils/editForm";

export function RoleStateEditModal({
  role,
  sessionId,
  scopeId,
  onClose,
}: RoleStateEditModalProps) {
  const { t } = useTranslation();
  const updateRole = useRoleState((s) => s.updateRole);

  const [name, setName] = useState(role.name ?? "");
  const [gender, setGender] = useState<RoleGender | "">(resolveGender(role) ?? "");
  const [location, setLocation] = useState(role.location ?? "");
  const [mood, setMood] = useState(role.mood ?? "");
  const [outfit, setOutfit] = useState(role.outfit ?? "");
  const [appearance, setAppearance] = useState(resolveAppearance(role) ?? "");
  const [tags, setTags] = useState<string[]>(parseTags(role.tags));
  const [tagDraft, setTagDraft] = useState("");
  const [attrs, setAttrs] = useState<AttrRow[]>(parseAttrs(role.attributes));
  const [meters, setMeters] = useState<MeterRow[]>(parseMeters(role.meters));

  const nsfw = role.nsfw;
  const semen = resolveSemen(nsfw);
  const [arousal, setArousal] = useState(
    typeof nsfw?.arousal === "number" ? nsfw.arousal : "",
  );
  const [wetness, setWetness] = useState(
    typeof nsfw?.wetness === "number" ? nsfw.wetness : "",
  );
  const [status, setStatus] = useState(nsfwStatus(nsfw ?? {}) ?? "");
  const [sensitiveText, setSensitiveText] = useState(
    spotsToText(nsfwSensitiveSpots(nsfw ?? {})),
  );
  const [semenTexture, setSemenTexture] = useState(semenText(semen, "texture") ?? "");
  const [semenExterior, setSemenExterior] = useState(semenText(semen, "exterior") ?? "");
  const [semenSwallowed, setSemenSwallowed] = useState(
    semenMl(semen, "swallowed") ?? ("" as number | ""),
  );
  const [semenVaginal, setSemenVaginal] = useState(
    semenMl(semen, "vaginal") ?? ("" as number | ""),
  );
  const [semenAnal, setSemenAnal] = useState(semenMl(semen, "anal") ?? ("" as number | ""));

  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, saving]);

  const addTag = () => {
    const next = tagDraft.trim();
    if (!next || tags.includes(next)) return;
    setTags([...tags, next]);
    setTagDraft("");
  };

  const buildRole = (): Role => {
    const next: Role = { ...role, id: role.id };

    const setOrClear = (key: keyof Role, value: string) => {
      const trimmed = value.trim();
      if (trimmed) (next as Record<string, unknown>)[key] = trimmed;
      else delete (next as Record<string, unknown>)[key];
    };

    setOrClear("name", name);
    setOrClear("location", location);
    setOrClear("mood", mood);
    setOrClear("outfit", outfit);
    setOrClear("appearance", appearance);

    if (gender === "male" || gender === "female") next.gender = gender;
    else delete next.gender;

    if (tags.length > 0) next.tags = tags;
    else delete next.tags;

    const attrObj: Record<string, number> = {};
    for (const row of attrs) {
      const k = row.key.trim();
      if (!k) continue;
      attrObj[k] = clamp01to100(Number(row.value));
    }
    if (Object.keys(attrObj).length > 0) next.attributes = attrObj;
    else delete next.attributes;

    const meterObj: Record<string, RoleMeter> = {};
    for (const row of meters) {
      const k = row.name.trim();
      if (!k) continue;
      meterObj[k] = {
        value: Number.isFinite(row.value) ? row.value : 0,
        max: Number.isFinite(row.max) && row.max > 0 ? row.max : 100,
      };
    }
    if (Object.keys(meterObj).length > 0) next.meters = meterObj;
    else delete next.meters;

    const nsfwNext: RoleNsfw = { ...(typeof role.nsfw === "object" && role.nsfw ? role.nsfw : {}) };
    // Drop legacy Chinese keys so English keys win after edit.
    delete nsfwNext["???"];
    delete nsfwNext["???"];
    delete nsfwNext["??"];
    delete nsfwNext["???"];
    delete nsfwNext["??"];

    if (arousal === "" || arousal == null) delete nsfwNext.arousal;
    else nsfwNext.arousal = clamp01to100(Number(arousal));

    if (wetness === "" || wetness == null) delete nsfwNext.wetness;
    else nsfwNext.wetness = clamp01to100(Number(wetness));

    const statusTrim = status.trim();
    if (statusTrim) nsfwNext.status = statusTrim;
    else delete nsfwNext.status;

    const spots = textToSpots(sensitiveText);
    if (spots.length > 0) nsfwNext.sensitive_spots = spots;
    else delete nsfwNext.sensitive_spots;

    const semenNext: Record<string, unknown> = {
      ...(nsfwNext.semen && typeof nsfwNext.semen === "object" ? nsfwNext.semen : {}),
    };
    delete semenNext["??"];
    delete semenNext["??"];
    delete semenNext["??"];
    delete semenNext["??"];

    const isMale = gender === "male";
    const isFemale = gender === "female";

    if (isMale || gender === "") {
      const tex = semenTexture.trim();
      if (tex) semenNext.texture = tex;
      else delete semenNext.texture;
    } else {
      delete semenNext.texture;
    }

    if (isFemale || gender === "") {
      const ext = semenExterior.trim();
      if (ext) semenNext.exterior = ext;
      else delete semenNext.exterior;

      const mlMap: Record<(typeof SEMEN_ML_KEYS)[number], number | ""> = {
        swallowed: semenSwallowed,
        vaginal: semenVaginal,
        anal: semenAnal,
      };
      for (const key of SEMEN_ML_KEYS) {
        const v = mlMap[key];
        if (v === "" || v == null) delete semenNext[key];
        else semenNext[key] = Number.isFinite(Number(v)) ? Number(v) : 0;
      }
    } else {
      delete semenNext.exterior;
      for (const key of SEMEN_ML_KEYS) delete semenNext[key];
    }

    if (Object.keys(semenNext).length > 0) nsfwNext.semen = semenNext;
    else delete nsfwNext.semen;

    if (Object.keys(nsfwNext).length > 0) next.nsfw = nsfwNext;
    else delete next.nsfw;

    return next;
  };

  const onSave = async () => {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      await updateRole(sessionId, scopeId, buildRole());
      onClose();
    } catch (e) {
      console.warn("[roleState] update failed", e);
      setError(t("roleState.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !saving) onClose();
      }}
    >
      <div
        className="modal rs-edit-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="rs-edit-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="rs-edit-title">{t("roleState.editTitle")}</h3>
          <button type="button" className="close" onClick={onClose} disabled={saving}>
            {t("common.close")}
          </button>
        </div>

        <div className="modal-body rs-edit-body">
          <section className="rs-edit-section">
            <h4 className="rs-edit-section-title">{t("roleState.sectionBasic")}</h4>
            <div className="rs-edit-grid">
              <label className="rs-edit-field">
                <span>{t("roleState.name")}</span>
                <input value={name} onChange={(e) => setName(e.target.value)} />
              </label>
              <label className="rs-edit-field">
                <span>{t("roleState.gender")}</span>
                <select
                  value={gender}
                  onChange={(e) => setGender(e.target.value as RoleGender | "")}
                >
                  <option value="">{t("roleState.genderUnset")}</option>
                  <option value="female">{t("roleState.genderFemale")}</option>
                  <option value="male">{t("roleState.genderMale")}</option>
                </select>
              </label>
              <label className="rs-edit-field">
                <span>{t("roleState.location")}</span>
                <input value={location} onChange={(e) => setLocation(e.target.value)} />
              </label>
              <label className="rs-edit-field">
                <span>{t("roleState.mood")}</span>
                <input value={mood} onChange={(e) => setMood(e.target.value)} />
              </label>
              <label className="rs-edit-field rs-edit-span2">
                <span>{t("roleState.outfit")}</span>
                <input value={outfit} onChange={(e) => setOutfit(e.target.value)} />
              </label>
              <label className="rs-edit-field rs-edit-span2">
                <span>{t("roleState.appearance")}</span>
                <textarea
                  rows={2}
                  value={appearance}
                  onChange={(e) => setAppearance(e.target.value)}
                />
              </label>
              <div className="rs-edit-field rs-edit-span2">
                <span>{t("roleState.tags")}</span>
                <div className="rs-edit-tags">
                  {tags.map((tg) => (
                    <span key={tg} className="rs-chip rs-edit-tag">
                      {tg}
                      <button
                        type="button"
                        className="rs-edit-tag-x"
                        aria-label={t("roleState.removeRow")}
                        onClick={() => setTags(tags.filter((x) => x !== tg))}
                      >
                        ×
                      </button>
                    </span>
                  ))}
                </div>
                <div className="rs-edit-row-add">
                  <input
                    value={tagDraft}
                    placeholder={t("roleState.addTag")}
                    onChange={(e) => setTagDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        addTag();
                      }
                    }}
                  />
                  <button type="button" className="btn" onClick={addTag}>
                    {t("roleState.addTag")}
                  </button>
                </div>
              </div>
            </div>
          </section>

          <section className="rs-edit-section">
            <h4 className="rs-edit-section-title">{t("roleState.sectionAttributes")}</h4>
            <div className="rs-edit-dyn">
              {attrs.map((row, i) => (
                <div key={i} className="rs-edit-dyn-row">
                  <input
                    value={row.key}
                    placeholder={t("roleState.attrKey")}
                    onChange={(e) => {
                      const next = [...attrs];
                      next[i] = { ...row, key: e.target.value };
                      setAttrs(next);
                    }}
                  />
                  <input
                    type="number"
                    min={0}
                    max={100}
                    value={row.value}
                    placeholder={t("roleState.attrValue")}
                    onChange={(e) => {
                      const next = [...attrs];
                      next[i] = { ...row, value: Number(e.target.value) };
                      setAttrs(next);
                    }}
                  />
                  <button
                    type="button"
                    className="btn rs-edit-remove"
                    onClick={() => setAttrs(attrs.filter((_, j) => j !== i))}
                  >
                    {t("roleState.removeRow")}
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="btn"
                onClick={() => setAttrs([...attrs, { key: "", value: 50 }])}
              >
                {t("roleState.addAttribute")}
              </button>
            </div>
          </section>

          <section className="rs-edit-section">
            <h4 className="rs-edit-section-title">{t("roleState.sectionMeters")}</h4>
            <div className="rs-edit-dyn">
              {meters.map((row, i) => (
                <div key={i} className="rs-edit-dyn-row rs-edit-meter-row">
                  <input
                    value={row.name}
                    placeholder={t("roleState.meterName")}
                    onChange={(e) => {
                      const next = [...meters];
                      next[i] = { ...row, name: e.target.value };
                      setMeters(next);
                    }}
                  />
                  <input
                    type="number"
                    value={row.value}
                    placeholder={t("roleState.meterValue")}
                    onChange={(e) => {
                      const next = [...meters];
                      next[i] = { ...row, value: Number(e.target.value) };
                      setMeters(next);
                    }}
                  />
                  <input
                    type="number"
                    value={row.max}
                    placeholder={t("roleState.meterMax")}
                    onChange={(e) => {
                      const next = [...meters];
                      next[i] = { ...row, max: Number(e.target.value) };
                      setMeters(next);
                    }}
                  />
                  <button
                    type="button"
                    className="btn rs-edit-remove"
                    onClick={() => setMeters(meters.filter((_, j) => j !== i))}
                  >
                    {t("roleState.removeRow")}
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="btn"
                onClick={() => setMeters([...meters, { name: "", value: 80, max: 100 }])}
              >
                {t("roleState.addMeter")}
              </button>
            </div>
          </section>

          <section className="rs-edit-section">
            <h4 className="rs-edit-section-title">{t("roleState.sectionNsfw")}</h4>
            <div className="rs-edit-grid">
              <label className="rs-edit-field">
                <span>{t("roleState.nsfwArousal")}</span>
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={arousal}
                  onChange={(e) =>
                    setArousal(e.target.value === "" ? "" : Number(e.target.value))
                  }
                />
              </label>
              <label className="rs-edit-field">
                <span>{t("roleState.nsfwWetness")}</span>
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={wetness}
                  onChange={(e) =>
                    setWetness(e.target.value === "" ? "" : Number(e.target.value))
                  }
                />
              </label>
              <label className="rs-edit-field rs-edit-span2">
                <span>{t("roleState.nsfwStatus")}</span>
                <input value={status} onChange={(e) => setStatus(e.target.value)} />
              </label>
              <label className="rs-edit-field rs-edit-span2">
                <span>{t("roleState.nsfwSensitive")}</span>
                <input
                  value={sensitiveText}
                  onChange={(e) => setSensitiveText(e.target.value)}
                  placeholder="a, b, c"
                />
              </label>

              {(gender === "male" || gender === "") && (
                <label className="rs-edit-field rs-edit-span2">
                  <span>{t("roleState.nsfwSemenTexture")}</span>
                  <input
                    value={semenTexture}
                    onChange={(e) => setSemenTexture(e.target.value)}
                  />
                </label>
              )}

              {(gender === "female" || gender === "") && (
                <>
                  <label className="rs-edit-field rs-edit-span2">
                    <span>{t("roleState.nsfwSemenExterior")}</span>
                    <input
                      value={semenExterior}
                      onChange={(e) => setSemenExterior(e.target.value)}
                    />
                  </label>
                  <label className="rs-edit-field">
                    <span>{t("roleState.nsfwSemenSwallowed")}</span>
                    <input
                      type="number"
                      min={0}
                      value={semenSwallowed}
                      onChange={(e) =>
                        setSemenSwallowed(
                          e.target.value === "" ? "" : Number(e.target.value),
                        )
                      }
                    />
                  </label>
                  <label className="rs-edit-field">
                    <span>{t("roleState.nsfwSemenVaginal")}</span>
                    <input
                      type="number"
                      min={0}
                      value={semenVaginal}
                      onChange={(e) =>
                        setSemenVaginal(
                          e.target.value === "" ? "" : Number(e.target.value),
                        )
                      }
                    />
                  </label>
                  <label className="rs-edit-field">
                    <span>{t("roleState.nsfwSemenAnal")}</span>
                    <input
                      type="number"
                      min={0}
                      value={semenAnal}
                      onChange={(e) =>
                        setSemenAnal(e.target.value === "" ? "" : Number(e.target.value))
                      }
                    />
                  </label>
                </>
              )}
            </div>
          </section>

          {error && <p className="rs-edit-error">{error}</p>}
        </div>

        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose} disabled={saving}>
            {t("roleState.cancel")}
          </button>
          <button type="button" className="btn primary" onClick={onSave} disabled={saving}>
            {saving ? t("roleState.saving") : t("roleState.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
