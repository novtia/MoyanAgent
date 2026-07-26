import { useSession } from "../../../../store/session";
import type { Role } from "../../../../store/roleState";
import { serializeRoleCite } from "../../mention/roleCite";
import type { RoleCitePayload } from "./constants";

export function roleToCitePayload(role: Role): RoleCitePayload {
  return {
    id: role.id,
    name: role.name,
  };
}

/** Insert a role-cite card token into the active composer prompt. */
export function quoteRoleToComposer(payload: RoleCitePayload) {
  const token = serializeRoleCite({
    id: payload.id,
    name: payload.name,
  });
  const { composer, setPrompt } = useSession.getState();
  const prompt = composer.prompt;
  const next = prompt.trim() ? `${token} ${prompt}` : `${token} `;
  setPrompt(next);
}
