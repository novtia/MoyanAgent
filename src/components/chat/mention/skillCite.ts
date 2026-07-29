/**
 * Skill cite chips — serialized as `@skill:{"id":"…","name":"…"}` in prompt
 * text, rendered as compact chips in the composer and message list.
 */

import i18n from "../../../i18n";

export const SKILL_CITE_PREFIX = "@skill:";

export interface SkillCiteRef {
  id: string;
  name?: string;
}

export function serializeSkillCite(ref: SkillCiteRef): string {
  const body: SkillCiteRef = { id: ref.id };
  const name = ref.name?.trim();
  if (name) body.name = name;
  return `${SKILL_CITE_PREFIX}${JSON.stringify(body)}`;
}

export function parseSkillCiteAt(
  text: string,
  atIndex: number,
): { id: string; name?: string; length: number } | null {
  if (text[atIndex] !== "@") return null;
  const rest = text.slice(atIndex);
  if (!rest.startsWith(SKILL_CITE_PREFIX)) return null;
  const jsonStart = atIndex + SKILL_CITE_PREFIX.length;
  if (text[jsonStart] !== "{") return null;

  let depth = 0;
  let end = -1;
  for (let i = jsonStart; i < text.length; i++) {
    const c = text[i];
    if (c === "{") depth += 1;
    else if (c === "}") {
      depth -= 1;
      if (depth === 0) {
        end = i + 1;
        break;
      }
    }
  }
  if (end < 0) return null;

  try {
    const obj = JSON.parse(text.slice(jsonStart, end)) as {
      id?: unknown;
      name?: unknown;
    };
    if (typeof obj.id !== "string" || !obj.id.trim()) return null;
    const name = typeof obj.name === "string" ? obj.name : undefined;
    return {
      id: obj.id.trim(),
      name: name?.trim() || undefined,
      length: end - atIndex,
    };
  } catch {
    return null;
  }
}

export function skillCiteDisplayName(ref: SkillCiteRef): string {
  return ref.name?.trim() || ref.id;
}

/** Contenteditable chip DOM (mirrors role cite). */
export function createSkillCiteNode(ref: SkillCiteRef): HTMLElement {
  const chip = document.createElement("span");
  chip.className = "composer-skill-cite";
  chip.contentEditable = "false";
  chip.dataset.skillId = ref.id;
  if (ref.name?.trim()) chip.dataset.skillName = ref.name.trim();
  chip.setAttribute("title", `${skillCiteDisplayName(ref)} (${ref.id})`);

  const eyebrow = document.createElement("span");
  eyebrow.className = "composer-skill-cite-eyebrow";
  eyebrow.textContent = i18n.t("composer.citeSkillLabel");

  const name = document.createElement("span");
  name.className = "composer-skill-cite-name";
  name.textContent = skillCiteDisplayName(ref);

  chip.appendChild(eyebrow);
  chip.appendChild(name);
  return chip;
}
