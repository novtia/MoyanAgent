export const MIN_WIDTH = 240;
export const MAX_WIDTH = 640;
export const DEFAULT_WIDTH = 340;
export const WIDTH_KEY = "atelier:right-panel-width";
/** Global right-panel open flag. Survives session switches and settings remounts. */
export const OPEN_KEY = "atelier:right-panel-open";
/** Brief global chrome-tab key; only used to migrate into a per-session key. */
export const TABS_KEY = "atelier:right-panel-tabs";
/** Per-session right-panel chrome tabs (not the reader file contents). */
export const TABS_KEY_PREFIX = "atelier:right-panel-tabs:";
