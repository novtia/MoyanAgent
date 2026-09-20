import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import { useSettings } from "../../../store/settings";
import type { AgentToolSpec } from "../../../types";
import { ToolDetail } from "./ToolDetail";
import { ToolsPane } from "./ToolsPane";

export function ToolsSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [tools, setTools] = useState<AgentToolSpec[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [selectedName, setSelectedName] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listAgentToolSpecs()
      .then((list) => {
        if (cancelled) return;
        setTools(list);
        setSelectedName((current) => current ?? list[0]?.name ?? null);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const disabledTools = settings?.disabled_tools ?? [];
  const toolDescriptions = settings?.tool_descriptions ?? {};
  const forcedTools = settings?.forced_tools ?? [];
  const echoContent = settings?.create_doc_echo_content ?? false;
  const replaceAllDefault = settings?.edit_replace_all_default ?? false;
  const paragraphLabels = settings?.read_paragraph_labels ?? false;
  const selected = tools?.find((tool) => tool.name === selectedName) ?? null;

  const filteredTools = useMemo(() => {
    if (!tools) return [];
    const q = search.trim().toLowerCase();
    if (!q) return tools;
    return tools.filter((tool) => {
      const desc = t(`agentFlow.toolDescriptions.${tool.name}`, {
        defaultValue: tool.description,
      });
      const custom = toolDescriptions[tool.name] ?? "";
      return (
        tool.name.toLowerCase().includes(q) ||
        desc.toLowerCase().includes(q) ||
        tool.description.toLowerCase().includes(q) ||
        custom.toLowerCase().includes(q)
      );
    });
  }, [tools, search, t, toolDescriptions]);

  const setEnabled = (name: string, enabled: boolean) => {
    const next = enabled
      ? disabledTools.filter((item) => item !== name)
      : disabledTools.includes(name)
        ? disabledTools
        : [...disabledTools, name];
    void update({ disabled_tools: next });
  };

  const setForced = (name: string, forced: boolean) => {
    const has = forcedTools.includes(name);
    if (forced === has) return;
    const next = forced
      ? [...forcedTools, name]
      : forcedTools.filter((item) => item !== name);
    void update({ forced_tools: next });
  };

  const setDescription = (name: string, text: string | null) => {
    const next = { ...toolDescriptions };
    const trimmed = text?.replace(/\r\n/g, "\n").trim() ?? "";
    if (trimmed === "") {
      if (!(name in next)) return;
      delete next[name];
    } else if (next[name] === trimmed) {
      return;
    } else {
      next[name] = trimmed;
    }
    void update({ tool_descriptions: next });
  };

  if (error) {
    return (
      <div className="model-service-card">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state">
            <span className="hint is-error">
              {t("settings.tools.loadError", { error })}
            </span>
          </div>
        </div>
      </div>
    );
  }

  if (!tools) {
    return (
      <div className="model-service-card">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state model-empty-state--center">
            {t("settings.tools.loading")}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="model-service-card">
      <div className="model-service-layout">
        <ToolsPane
          tools={tools}
          filteredTools={filteredTools}
          selectedName={selected?.name}
          disabledTools={disabledTools}
          forcedTools={forcedTools}
          descriptions={toolDescriptions}
          search={search}
          onSearchChange={setSearch}
          onSelect={(tool) => setSelectedName(tool.name)}
          onSetEnabled={setEnabled}
        />
        <ToolDetail
          key={selected?.name ?? "none"}
          selected={selected}
          enabled={selected ? !disabledTools.includes(selected.name) : false}
          forced={selected ? forcedTools.includes(selected.name) : false}
          echoContent={echoContent}
          replaceAllDefault={replaceAllDefault}
          paragraphLabels={paragraphLabels}
          novelai={settings?.novelai}
          descriptions={toolDescriptions}
          onSetEnabled={setEnabled}
          onSetForced={setForced}
          onSetDescription={setDescription}
          onSetEchoContent={(next) => {
            void update({ create_doc_echo_content: next });
          }}
          onSetReplaceAllDefault={(next) => {
            void update({ edit_replace_all_default: next });
          }}
          onSetParagraphLabels={(next) => {
            void update({ read_paragraph_labels: next });
          }}
          onSetNovelai={(next) => {
            void update({ novelai: next });
          }}
        />
      </div>
    </div>
  );
}
