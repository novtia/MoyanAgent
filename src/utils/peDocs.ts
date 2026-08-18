/** Android/iOS document-library platform (not a 900px desktop shrink). */
export function isPePlatform(): boolean {
  if (typeof document === "undefined") return false;
  const p = document.documentElement.dataset.platform;
  return p === "android" || p === "ios";
}

export const PE_DOC_EXTENSIONS = [".md", ".txt", ".markdown"] as const;

export function isPeDocFile(path: string): boolean {
  const lower = path.toLowerCase();
  return PE_DOC_EXTENSIONS.some((ext) => lower.endsWith(ext));
}
