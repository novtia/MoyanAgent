import { api } from "../../../../../api/tauri";
import { useReader } from "../../../../../store/reader";
import {
  baseName,
  joinPath,
  parentDir,
  uniqueName,
  useFileExplorer,
} from "../../../../../store/fileExplorer";
import {
  READER_FILE_DRAG_TYPE,
  type ReaderDragItem,
} from "../../../../../utils/readerDrag";

function normPath(p: string): string {
  return p.replace(/[/\\]+$/, "").replace(/\\/g, "/").toLowerCase();
}

function isSamePath(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return normPath(a) === normPath(b);
}

function isWithin(parent: string, child: string): boolean {
  const p = normPath(parent);
  const c = normPath(child);
  return c === p || c.startsWith(`${p}/`);
}

export type TreeDropKind = "internal" | "external" | null;

export interface ImportDropResult {
  kind: "internal" | "external";
  moved: number;
  imported: number;
  skippedNoPath: number;
  errors: string[];
}

/** True when the drag payload can be accepted by the file tree. */
export function isTreeDroppable(dt: DataTransfer | null): boolean {
  if (!dt) return false;
  const types = Array.from(dt.types || []);
  if (types.includes(READER_FILE_DRAG_TYPE)) return true;
  return types.includes("Files");
}

export function resolveTreeDropTargetDir(opts: {
  root: string;
  entryPath?: string | null;
  isDir?: boolean;
}): string {
  const { root, entryPath, isDir } = opts;
  if (!entryPath) return root;
  if (isDir) return entryPath;
  return parentDir(entryPath) ?? root;
}

function nativeFilePath(file: File): string | null {
  const path = (file as File & { path?: string }).path?.trim();
  return path || null;
}

/** Parse `file://` / plain absolute paths from text/uri-list or text/plain. */
function pathsFromUriList(raw: string): string[] {
  const out: string[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    if (trimmed.startsWith("file:")) {
      try {
        const url = new URL(trimmed);
        let p = decodeURIComponent(url.pathname);
        // Windows: `/C:/Users/...` → `C:\Users\...`
        if (/^\/[A-Za-z]:\//.test(p)) {
          p = p.slice(1).replace(/\//g, "\\");
        } else if (url.hostname) {
          // UNC: file://server/share → \\server\share
          p = `\\\\${url.hostname}${p.replace(/\//g, "\\")}`;
        }
        if (p) out.push(p);
      } catch {
        /* ignore */
      }
      continue;
    }
    // Plain absolute path (Explorer sometimes puts this in text/plain).
    if (/^[A-Za-z]:[\\/]/.test(trimmed) || trimmed.startsWith("\\\\")) {
      out.push(trimmed);
    }
  }
  return out;
}

function collectExternalPaths(dt: DataTransfer): {
  pathItems: { path: string; name: string }[];
  byteFiles: File[];
} {
  const pathItems: { path: string; name: string }[] = [];
  const seen = new Set<string>();
  const pushPath = (path: string, nameHint?: string) => {
    const clean = path.trim();
    if (!clean) return;
    const key = normPath(clean);
    if (seen.has(key)) return;
    seen.add(key);
    pathItems.push({ path: clean, name: nameHint || baseName(clean) || "file" });
  };

  for (const file of Array.from(dt.files || [])) {
    const path = nativeFilePath(file);
    if (path) pushPath(path, file.name);
  }

  for (const type of ["text/uri-list", "text/plain"] as const) {
    try {
      const raw = dt.getData(type);
      if (raw) {
        for (const p of pathsFromUriList(raw)) pushPath(p);
      }
    } catch {
      /* ignore */
    }
  }

  // Files without a resolvable OS path — write via bytes (folders can't use this).
  const byteFiles = Array.from(dt.files || []).filter((file) => {
    const path = nativeFilePath(file);
    if (path) return false;
    // Empty zero-size nameless stubs are often directory drops without contents.
    if (!file.name && file.size === 0) return false;
    return true;
  });

  return { pathItems, byteFiles };
}

function parseInternalDrag(dt: DataTransfer): ReaderDragItem[] | null {
  const raw = dt.getData(READER_FILE_DRAG_TYPE);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as ReaderDragItem[];
    if (!Array.isArray(parsed) || parsed.length === 0) return null;
    return parsed.filter((item) => item && typeof item.path === "string" && item.path);
  } catch {
    return null;
  }
}

/** Classify drag for dropEffect / highlight (types only — getData may be empty on dragover). */
export function peekTreeDropKind(dt: DataTransfer | null): TreeDropKind {
  if (!dt) return null;
  const types = Array.from(dt.types || []);
  if (types.includes(READER_FILE_DRAG_TYPE)) return "internal";
  if (types.includes("Files")) return "external";
  return null;
}

/**
 * Handle a drop onto a file-tree target directory.
 * Internal tree drags move; OS files/folders are copied in with unique names.
 * When `File.path` is missing (Tauri `dragDropEnabled: false`), falls back to
 * uri-list paths or writing file bytes into the project.
 */
export async function handleTreeDrop(opts: {
  sessionId: string;
  targetDir: string;
  dataTransfer: DataTransfer;
}): Promise<ImportDropResult | null> {
  const { sessionId, targetDir, dataTransfer } = opts;
  const internal = parseInternalDrag(dataTransfer);
  if (internal && internal.length > 0) {
    const store = useFileExplorer.getState();
    let moved = 0;
    if (store.sessionId === sessionId) {
      moved = await store.dropMove(
        internal.map((item) => item.path),
        targetDir,
      );
    } else {
      const remaps: { from: string; to: string }[] = [];
      for (const item of internal) {
        const from = item.path;
        if (isSamePath(parentDir(from), targetDir)) continue;
        if (isWithin(from, targetDir)) continue;
        const target = joinPath(targetDir, baseName(from));
        try {
          await api.renameProjectPath(sessionId, from, target);
          remaps.push({ from, to: target });
          moved += 1;
        } catch (err) {
          return {
            kind: "internal",
            moved,
            imported: 0,
            skippedNoPath: 0,
            errors: [err instanceof Error ? err.message : String(err)],
          };
        }
      }
      if (remaps.length > 0) useReader.getState().remapPaths(remaps);
      if (moved > 0) store.bumpTree();
    }
    return {
      kind: "internal",
      moved,
      imported: 0,
      skippedNoPath: 0,
      errors: [],
    };
  }

  const { pathItems, byteFiles } = collectExternalPaths(dataTransfer);
  if (pathItems.length === 0 && byteFiles.length === 0) {
    const hadFiles = (dataTransfer.files?.length ?? 0) > 0;
    if (!hadFiles) return null;
    return {
      kind: "external",
      moved: 0,
      imported: 0,
      skippedNoPath: dataTransfer.files.length,
      errors: [],
    };
  }

  let existing = new Set<string>();
  try {
    const entries = await api.listProjectDir(sessionId, targetDir);
    existing = new Set(entries.map((e) => e.name));
  } catch {
    // Proceed; backend will reject collisions.
  }

  let imported = 0;
  const errors: string[] = [];

  for (const item of pathItems) {
    const finalName = uniqueName(existing, item.name);
    existing.add(finalName);
    const destPath = joinPath(targetDir, finalName);
    try {
      await api.importExternalPathToProject(sessionId, item.path, destPath);
      imported += 1;
    } catch (err) {
      errors.push(`${item.name}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  for (const file of byteFiles) {
    const name = file.name?.trim() || "untitled";
    const finalName = uniqueName(existing, name);
    existing.add(finalName);
    const destPath = joinPath(targetDir, finalName);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      await api.writeProjectFileBytes(sessionId, destPath, bytes);
      imported += 1;
    } catch (err) {
      errors.push(`${name}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  return {
    kind: "external",
    moved: 0,
    imported,
    skippedNoPath: 0,
    errors,
  };
}
