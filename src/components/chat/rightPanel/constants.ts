export const MIN_WIDTH = 240;
export const MAX_WIDTH = 640;
export const DEFAULT_WIDTH = 340;
export const WIDTH_KEY = "atelier:right-panel-width";
/** Global right-panel open flag. Survives session switches and settings remounts. */
export const OPEN_KEY = "atelier:right-panel-open";
/** Legacy global chrome-tab key. Only purged now — never read back. */
export const LEGACY_TABS_KEY = "atelier:right-panel-tabs";
/** Per-session right-panel chrome tabs (not the reader file contents). */
export const TABS_KEY_PREFIX = "atelier:right-panel-tabs:";
/** Stored tab payload schema version. */
export const TABS_SCHEMA_VERSION = 2;
/** Max simultaneously mounted tab panes (keep-alive budget, LRU beyond this). */
export const MAX_MOUNTED_PANES = 8;
