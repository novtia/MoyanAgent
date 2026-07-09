/** Strip Windows extended-length prefixes (`\\?\`, `\\?\UNC\`) for display/storage. */
export function sanitizeFsPath(path: string): string {
  const t = path.trim();
  if (/^\\\\\?\\UNC\\/i.test(t)) return `\\\\${t.slice(8)}`;
  if (/^\\\\\?\\/.test(t)) return t.slice(4);
  return t;
}
