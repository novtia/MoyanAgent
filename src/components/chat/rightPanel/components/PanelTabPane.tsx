import { usePanelTab } from "../context/PanelTabContext";
import { GalleryContent } from "../gallery";
import { AgentFlowPanel } from "../agentFlow";
import { RoleStatePanel } from "../roleState";
import { ReaderWorkspace } from "../reader";
import { TypePicker } from "./TypePicker";
import type { ImageRefAbs } from "../../../../types";
import type { PanelTab, PickerKind } from "../types";

export interface PanelTabPaneProps {
  tab: PanelTab;
  /** Right panel itself is expanded (not collapsed). */
  panelOpen: boolean;
  showReader: boolean;
  onPickKind: (kind: PickerKind) => void;
  onPreviewImage: (img: ImageRefAbs) => void;
  onOpenFile: (path: string) => void;
}

/**
 * Body of a single tab. Every pane is an independent instance: two reader tabs
 * (or two galleries) hold separate component state and never share it.
 *
 * `panelOpen && isActive` is what a feature should treat as "visible": hidden
 * panes stay mounted, so they must not measure layout or steal focus.
 */
export function PanelTabPane({
  tab,
  panelOpen,
  showReader,
  onPickKind,
  onPreviewImage,
  onOpenFile,
}: PanelTabPaneProps) {
  const { isActive } = usePanelTab();
  const visible = panelOpen && isActive;

  switch (tab.kind) {
    case "gallery":
      return <GalleryContent open={visible} onPreviewImage={onPreviewImage} />;
    case "role-state":
      return <RoleStatePanel open={visible} />;
    case "agent-flow":
      return <AgentFlowPanel open={visible} />;
    case "reader":
      return <ReaderWorkspace path={tab.path ?? null} onOpenFile={onOpenFile} />;
    default:
      return (
        <TypePicker tab={visible ? 0 : -1} showReader={showReader} onPick={onPickKind} />
      );
  }
}
