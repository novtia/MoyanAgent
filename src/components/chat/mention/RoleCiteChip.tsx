import { useTranslation } from "react-i18next";

import { roleCiteDisplayName, type RoleCiteRef } from "./roleCite";

/** Static role-cite card — React counterpart of {@link createRoleCiteNode}. */
export function RoleCiteChip({ id, name }: RoleCiteRef) {
  const { t } = useTranslation();
  const displayName = roleCiteDisplayName({ id, name });
  return (
    <span
      className="composer-role-cite composer-role-cite--static"
      title={`${displayName} (${id})`}
    >
      <span className="composer-role-cite-body">
        <span className="composer-role-cite-eyebrow">{t("roleState.citeCardLabel")}</span>
        <span className="composer-role-cite-name">{displayName}</span>
        <span className="composer-role-cite-id">{id}</span>
      </span>
    </span>
  );
}
