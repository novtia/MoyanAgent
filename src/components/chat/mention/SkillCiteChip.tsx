import { useTranslation } from "react-i18next";
import { skillCiteDisplayName, type SkillCiteRef } from "./skillCite";

export function SkillCiteChip({ id, name }: SkillCiteRef) {
  const { t } = useTranslation();
  return (
    <span
      className="composer-skill-cite composer-skill-cite--static"
      title={`${skillCiteDisplayName({ id, name })} (${id})`}
    >
      <span className="composer-skill-cite-eyebrow">{t("composer.citeSkillLabel")}</span>
      <span className="composer-skill-cite-name">
        {skillCiteDisplayName({ id, name })}
      </span>
    </span>
  );
}
