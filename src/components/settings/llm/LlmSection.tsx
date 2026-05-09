import { ApiSettingsCard } from "./ApiSettingsCard";
import { GenerationDefaultsCard } from "./GenerationDefaultsCard";
import { HistoryTurnsCard } from "./HistoryTurnsCard";
import { ModelParamsCard } from "./ModelParamsCard";
import { ModelSettingsCard } from "./ModelSettingsCard";
import { SystemPromptCard } from "./SystemPromptCard";

export function LlmSection() {
  return (
    <>
      <ApiSettingsCard />
      <ModelSettingsCard />
      <GenerationDefaultsCard />
      <SystemPromptCard />
      <HistoryTurnsCard />
      <ModelParamsCard />
    </>
  );
}
