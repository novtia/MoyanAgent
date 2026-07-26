/**
 * Role-state cite chips — serialized as `@role:{"id":"…","name":"…"}` in prompt
 * text, rendered as compact archive cards in the composer and message list.
 */

import i18n from "../../../i18n";

export const ROLE_CITE_PREFIX = "@role:";

export interface RoleCiteRef {
  id: string;
  name?: string;
}

export function serializeRoleCite(ref: RoleCiteRef): string {
  const body: RoleCiteRef = { id: ref.id };
  const name = ref.name?.trim();
  if (name) body.name = name;
  return `${ROLE_CITE_PREFIX}${JSON.stringify(body)}`;
}

export function parseRoleCiteAt(
  text: string,
  atIndex: number,
): { id: string; name?: string; length: number } | null {
  if (text[atIndex] !== "@") return null;
  const rest = text.slice(atIndex);
  if (!rest.startsWith(ROLE_CITE_PREFIX)) return null;
  const jsonStart = atIndex + ROLE_CITE_PREFIX.length;
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

export function roleCiteDisplayName(ref: RoleCiteRef): string {
  return ref.name?.trim() || ref.id;
}

/** Contenteditable chip DOM (mirrors {@link createMentionNode}). */
export function createRoleCiteNode(ref: RoleCiteRef): HTMLElement {
  const chip = document.createElement("span");
  chip.className = "composer-role-cite";
  chip.contentEditable = "false";
  chip.dataset.roleId = ref.id;
  if (ref.name?.trim()) chip.dataset.roleName = ref.name.trim();
  chip.setAttribute("title", `${roleCiteDisplayName(ref)} (${ref.id})`);

  const eyebrow = document.createElement("span");
  eyebrow.className = "composer-role-cite-eyebrow";
  eyebrow.textContent = i18n.t("roleState.citeCardLabel");

  const name = document.createElement("span");
  name.className = "composer-role-cite-name";
  name.textContent = roleCiteDisplayName(ref);

  const id = document.createElement("span");
  id.className = "composer-role-cite-id";
  id.textContent = ref.id;

  const body = document.createElement("span");
  body.className = "composer-role-cite-body";
  body.appendChild(eyebrow);
  body.appendChild(name);
  body.appendChild(id);
  chip.appendChild(body);

  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "composer-mention-remove";
  remove.textContent = "×";
  remove.tabIndex = -1;
  chip.appendChild(remove);

  return chip;
}
