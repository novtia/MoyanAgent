import type { WebSearchHit } from "../../../types";
import type { ListFilesEntry } from "./types";
import { parseListFilesOutput } from "./utils";

function asObj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : null;
}

function asStr(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function asNum(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

function asBool(v: unknown): boolean | undefined {
  return typeof v === "boolean" ? v : undefined;
}

export type WriteOutput = {
  path: string;
  created?: boolean;
  text?: string;
  chars?: number;
  lines?: number;
  bytes?: number;
};

export function parseWriteOutput(output: unknown): WriteOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const path = asStr(o.path);
  if (!path) return null;
  return {
    path,
    created: asBool(o.created),
    text: asStr(o.text),
    chars: asNum(o.chars),
    lines: asNum(o.lines),
    bytes: asNum(o.bytes),
  };
}

export type BashOutput = {
  command?: string;
  exit_code: number;
  stdout: string;
  stderr: string;
  stdout_truncated?: boolean;
  stderr_truncated?: boolean;
};

export function parseBashOutput(output: unknown): BashOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const exit = asNum(o.exit_code);
  if (exit == null) return null;
  return {
    command: asStr(o.command),
    exit_code: exit,
    stdout: asStr(o.stdout) ?? "",
    stderr: asStr(o.stderr) ?? "",
    stdout_truncated: asBool(o.stdout_truncated),
    stderr_truncated: asBool(o.stderr_truncated),
  };
}

export type GrepMatch = {
  paragraph?: number;
  label?: string;
  occurrences?: number;
  text: string;
};

export type GrepFileResult = {
  path: string;
  matches: GrepMatch[];
};

export type GrepOutput = {
  path?: string;
  query: string;
  total_matches?: number;
  truncated?: boolean;
  files_capped?: boolean;
  files: GrepFileResult[];
};

function parseMatches(raw: unknown): GrepMatch[] {
  if (!Array.isArray(raw)) return [];
  const out: GrepMatch[] = [];
  for (const row of raw) {
    const m = asObj(row);
    if (!m) continue;
    const text = asStr(m.text);
    if (text == null) continue;
    out.push({
      paragraph: asNum(m.paragraph),
      label: asStr(m.label),
      occurrences: asNum(m.occurrences),
      text,
    });
  }
  return out;
}

export function parseGrepOutput(output: unknown): GrepOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const query = asStr(o.query);
  if (!query) return null;

  // Directory mode: { results: [{ path, matches }] }
  if (Array.isArray(o.results)) {
    const files: GrepFileResult[] = [];
    for (const row of o.results) {
      const f = asObj(row);
      if (!f) continue;
      const path = asStr(f.path);
      if (!path) continue;
      files.push({ path, matches: parseMatches(f.matches) });
    }
    return {
      path: asStr(o.path),
      query,
      total_matches: asNum(o.total_matches),
      truncated: asBool(o.truncated),
      files_capped: asBool(o.files_capped),
      files,
    };
  }

  // Single-file mode: { path, matches }
  const path = asStr(o.path) ?? "";
  return {
    path,
    query,
    total_matches: asNum(o.total_matches),
    truncated: asBool(o.truncated),
    files: [{ path, matches: parseMatches(o.matches) }],
  };
}

export type WebSearchToolOutput = {
  backend: string;
  query: string;
  hits: WebSearchHit[];
  message?: string;
};

export function parseWebSearchOutput(output: unknown): WebSearchToolOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const query = asStr(o.query) ?? "";
  const backend = asStr(o.backend) ?? "";
  const rows = Array.isArray(o.results)
    ? o.results
    : Array.isArray(o.hits)
      ? o.hits
      : null;
  if (!rows) return null;
  const hits: WebSearchHit[] = [];
  for (const row of rows) {
    const h = asObj(row);
    if (!h) continue;
    const title = asStr(h.title) ?? "";
    const url = asStr(h.url) ?? "";
    if (!title && !url) continue;
    hits.push({
      title,
      url,
      snippet: asStr(h.snippet) ?? "",
      published: asStr(h.published) ?? null,
      source: asStr(h.source) ?? backend,
    });
  }
  return {
    backend,
    query,
    hits,
    message: asStr(o.message),
  };
}

export type WebFetchOutput = {
  url: string;
  title?: string;
  text: string;
  truncated?: boolean;
};

export function parseWebFetchOutput(output: unknown): WebFetchOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const url = asStr(o.url);
  const text = asStr(o.text);
  if (!url || text == null) return null;
  return {
    url,
    title: asStr(o.title),
    text,
    truncated: asBool(o.truncated),
  };
}

export type AgentCompleted = {
  status: "completed";
  agent_id?: string;
  task_id?: string;
  text?: string;
  tool_calls?: number;
  child_session_id?: string;
};

export type AgentAsyncLaunched = {
  status: "async_launched";
  agent_id?: string;
  task_id?: string;
  output_file?: string;
  child_session_id?: string;
};

export type AgentRunning = {
  status: "running";
  child_session_id: string;
};

export type AgentOutput = AgentCompleted | AgentAsyncLaunched | AgentRunning;

export function parseAgentOutput(output: unknown): AgentOutput | null {
  const o = asObj(output);
  if (!o) return null;
  const status = asStr(o.status);
  const child_session_id = asStr(o.child_session_id);
  if (status === "running" && child_session_id) {
    return { status: "running", child_session_id };
  }
  if (status === "completed") {
    return {
      status: "completed",
      agent_id: asStr(o.agent_id),
      task_id: asStr(o.task_id),
      text: asStr(o.text),
      tool_calls: asNum(o.tool_calls),
      child_session_id,
    };
  }
  if (status === "async_launched") {
    return {
      status: "async_launched",
      agent_id: asStr(o.agent_id),
      task_id: asStr(o.task_id),
      output_file: asStr(o.output_file),
      child_session_id,
    };
  }
  return null;
}

export type ListFilesParsed = {
  path?: string;
  truncated?: boolean;
  entries: ListFilesEntry[];
};

export function parseListFilesToolOutput(output: unknown): ListFilesParsed | null {
  const entries = parseListFilesOutput(output);
  if (!entries) return null;
  const o = asObj(output);
  return {
    path: o ? asStr(o.path) : undefined,
    truncated: o ? asBool(o.truncated) : undefined,
    entries,
  };
}
