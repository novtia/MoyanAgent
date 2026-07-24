import type { RoleGender } from "../../../../../store/roleState";

/** i18n label for a known nsfw / semen field key; falls back to the raw key. */
export function nsfwLabel(t: (k: string) => string, key: string, semen = false): string {
  const map: Record<string, string> = semen
    ? {
        texture: "roleState.nsfwSemenTexture",
        exterior: "roleState.nsfwSemenExterior",
        swallowed: "roleState.nsfwSemenSwallowed",
        vaginal: "roleState.nsfwSemenVaginal",
        anal: "roleState.nsfwSemenAnal",
      }
    : {
        arousal: "roleState.nsfwArousal",
        wetness: "roleState.nsfwWetness",
        status: "roleState.nsfwStatus",
        sensitive_spots: "roleState.nsfwSensitive",
      };
  const i18nKey = map[key];
  return i18nKey ? t(i18nKey) : key;
}

export function genderLabel(t: (k: string) => string, gender: RoleGender): string {
  return gender === "male" ? t("roleState.genderMale") : t("roleState.genderFemale");
}
