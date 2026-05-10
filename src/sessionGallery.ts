import type { ImageRefAbs, SessionWithMessagesAbs } from "./types";

/** Same ordering and dedupe rules as the session gallery sidebar. */
export function collectSessionGalleryImages(active: SessionWithMessagesAbs | null): ImageRefAbs[] {
  if (!active) return [];
  const all: ImageRefAbs[] = [];
  const seenKey = new Set<string>();
  const seenContent = new Set<string>();
  const contentKey = (img: ImageRefAbs) => {
    if (img.bytes && img.width && img.height) {
      return `${img.bytes}|${img.width}x${img.height}|${img.mime}`;
    }
    return null;
  };
  const push = (img: ImageRefAbs) => {
    const idKey = img.abs_path || img.id;
    if (idKey && seenKey.has(idKey)) return;
    const ck = contentKey(img);
    if (ck && seenContent.has(ck)) return;
    if (idKey) seenKey.add(idKey);
    if (img.id) seenKey.add(img.id);
    if (ck) seenContent.add(ck);
    all.push(img);
  };
  for (let i = 0; i < active.messages.length; i++) {
    const m = active.messages[i];
    const ins = m.images.filter((x) => x.role === "input");
    const outs = m.images.filter((x) => x.role === "output" || x.role === "edited");
    ins.forEach(push);
    outs.forEach(push);
  }
  return all;
}

export function indexOfImageInGallery(items: ImageRefAbs[], img: ImageRefAbs): number {
  const byId = items.findIndex((x) => x.id === img.id);
  if (byId >= 0) return byId;
  return items.findIndex((x) => x.abs_path === img.abs_path);
}
