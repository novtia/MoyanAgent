/**
 * Project-level parameter settings modal.
 * All sessions in the project inherit these settings instead of their own.
 * UI is provided by the shared ScopeConfigModal (left nav + content panel).
 */
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useProject } from "../../store/project";
import type { Project } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";
import { ScopeConfigModal } from "./ScopeConfigModal";

interface ProjectConfigModalProps {
  project: Project;
  onClose: () => void;
}

export function ProjectConfigModal({ project, onClose }: ProjectConfigModalProps) {
  const updateConfig = useProject((s) => s.updateConfig);
  const updatePath = useProject((s) => s.updatePath);

  return (
    <ScopeConfigModal
      title="项目设置"
      subtitle={project.name}
      scopeNote="以下参数应用于该项目下所有会话，会话自身的参数设置不再生效。"
      promptPlaceholder="应用于项目所有会话；留空则不发送 system 提示词。"
      paramsHint="应用于项目所有会话的请求体；留空则不发送对应字段。"
      initial={{
        systemPrompt: project.system_prompt ?? "",
        historyTurns: project.history_turns ?? 10,
        llmParams: { ...EMPTY_MODEL_PARAMS, ...(project.llm_params ?? EMPTY_MODEL_PARAMS) },
      }}
      pathField={{
        value: project.path,
        onBrowse: async () => {
          const result = await openDialog({
            directory: true,
            multiple: false,
            title: "选择项目文件夹",
          });
          return typeof result === "string" ? result : null;
        },
        onSave: async (path) => {
          await updatePath(project.id, path);
        },
      }}
      onSave={async (systemPrompt, historyTurns, llmParams) => {
        await updateConfig(
          project.id,
          systemPrompt,
          historyTurns,
          llmParams,
          null, // context_window 由模型目录自动管理，不在此处手动设置
        );
      }}
      onClose={onClose}
    />
  );
}
