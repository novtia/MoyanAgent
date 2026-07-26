import { FileTypeIcon } from "../../../utils/fileIcons";
import { mentionFileIconName } from "./core";

/** File-type icon for mention chips — shared with the reader file tree. */
export function MentionIcon({ path, isDir }: { path: string; isDir?: boolean }) {
  return (
    <FileTypeIcon
      name={mentionFileIconName(path, isDir)}
      className="composer-mention-icon"
    />
  );
}
