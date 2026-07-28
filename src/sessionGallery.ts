import type { ImageRefAbs, SessionWithMessagesAbs } from "./types";

/** Deduplicate gallery media rows (from messages window or list_session_media). */
export function dedupeGalleryMedia(items: ImageRefAbs[]): ImageRefAbs[] {
  const all: ImageRefAbs[] = [];
  const seenKey = new Set<string>();
  const seenContent = new Set<string>();
  const contentKey = (img: ImageRefAbs) => {
    if (img.bytes && img.width && img.height) {
      return `${img.bytes}|${img.width}x${img.height}|${img.mime}`;
    }
    return null;
  };
  for (const img of items) {
    const idKey = img.abs_path || img.id;
    if (idKey && seenKey.has(idKey)) continue;
    const ck = contentKey(img);
    if (ck && seenContent.has(ck)) continue;
    if (idKey) seenKey.add(idKey);
    if (img.id) seenKey.add(img.id);
    if (ck) seenContent.add(ck);
    all.push(img);
  }
  return all;
}

/** Images plus generated video outputs, in message order with content dedupe. */
export function collectSessionGalleryMedia(
  active: SessionWithMessagesAbs | null,
  sessionMedia?: ImageRefAbs[] | null,
): ImageRefAbs[] {
  if (sessionMedia && sessionMedia.length > 0) {
    return dedupeGalleryMedia(
      sessionMedia.filter(
        (x) =>
          (x.role === "input" && x.mime.startsWith("image/")) ||
          ((x.role === "output" || x.role === "edited") &&
            (x.mime.startsWith("image/") || x.mime.startsWith("video/"))),
      ),
    );
  }
  if (!active) return [];
  const raw: ImageRefAbs[] = [];
  for (let i = 0; i < active.messages.length; i++) {
    const m = active.messages[i];
    raw.push(
      ...m.images.filter(
        (x) => x.role === "input" && x.mime.startsWith("image/"),
      ),
    );
    raw.push(
      ...m.images.filter(
        (x) =>
          (x.role === "output" || x.role === "edited") &&
          (x.mime.startsWith("image/") || x.mime.startsWith("video/")),
      ),
    );
  }
  return dedupeGalleryMedia(raw);
}

export function collectSessionGalleryImages(
  active: SessionWithMessagesAbs | null,
  sessionMedia?: ImageRefAbs[] | null,
): ImageRefAbs[] {
  return collectSessionGalleryMedia(active, sessionMedia).filter((item) =>
    item.mime.startsWith("image/"),
  );
}

export function indexOfMediaInGallery(
  items: ImageRefAbs[],
  img: ImageRefAbs,
): number {
  const byId = items.findIndex((x) => x.id === img.id);
  if (byId >= 0) return byId;
  return items.findIndex((x) => x.abs_path === img.abs_path);
}

export const indexOfImageInGallery = indexOfMediaInGallery;
