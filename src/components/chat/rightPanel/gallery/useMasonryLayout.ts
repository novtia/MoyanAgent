import { useMemo } from "react";
import type { ImageRefAbs } from "../../../../types";
import { MASONRY_GAP, MASONRY_MIN_COL } from "./constants";
import type { MasonryItem } from "./types";

export function useMasonryLayout(media: ImageRefAbs[], innerWidth: number) {
  return useMemo(() => {
    if (innerWidth <= 0 || media.length === 0) {
      return { items: [] as MasonryItem[], total: 0 };
    }
    const cols = Math.max(1, Math.floor((innerWidth + MASONRY_GAP) / (MASONRY_MIN_COL + MASONRY_GAP)));
    const colW = (innerWidth - MASONRY_GAP * (cols - 1)) / cols;
    const heights = new Array(cols).fill(0);
    const items = media.map((img) => {
      const aspect =
        img.width && img.height && img.width > 0
          ? img.height / img.width
          : img.mime.startsWith("video/")
            ? 9 / 16
            : 1;
      const h = Math.max(40, Math.round(colW * aspect));
      let idx = 0;
      let min = heights[0];
      for (let i = 1; i < cols; i++) {
        if (heights[i] < min) {
          min = heights[i];
          idx = i;
        }
      }
      const x = idx * (colW + MASONRY_GAP);
      const y = heights[idx];
      heights[idx] = y + h + MASONRY_GAP;
      return { img, x, y, w: colW, h };
    });
    const total = Math.max(0, ...heights) - MASONRY_GAP;
    return { items, total };
  }, [media, innerWidth]);
}
