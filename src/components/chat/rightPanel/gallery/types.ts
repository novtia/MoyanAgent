import type { ImageRefAbs } from "../../../../types";

export interface GalleryContentProps {
  open: boolean;
  onPreviewImage: (img: ImageRefAbs) => void;
}

export interface MasonryItem {
  img: ImageRefAbs;
  x: number;
  y: number;
  w: number;
  h: number;
}
