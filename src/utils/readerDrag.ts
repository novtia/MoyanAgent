/** DataTransfer MIME for dragging project files into the composer as @ mentions. */
export const READER_FILE_DRAG_TYPE = "application/x-moyan-reader-file";

/** One dragged file/folder entry payload. */
export interface ReaderDragItem {
  path: string;
  isDir: boolean;
}
