// Mirror of skill/schema.json (the `--json` answers); `/api/*` returns the same.

export type Kind = "file" | "dir" | "symlink" | "other" | "mount";

export interface Header {
  host: string;
  os: string;
  generation: number;
  prev_generation: number | null;
  scanned_at: number;
  age_seconds: number;
  schema_version: number;
  snapshot_id: string;
  incomplete: boolean;
  unknown_subtrees: string[];
}

export interface MountOut {
  fs_type: string;
  used: number | null;
  total: number | null;
  root: string | null;
}

export interface Entry {
  path: string;
  path_raw?: string;
  name: string;
  kind: Kind;
  allocated: number;
  apparent: number;
  mtime: number;
  newest_mtime: number;
  nlink: number;
  items: number;
  errors: number;
  scanned_at: number;
  denied?: boolean;
  hardlink_alias?: boolean;
  hardlink_owner?: string;
  mount?: MountOut;
}

export interface LsResponse extends Header {
  entry: Entry;
  entries: Entry[];
  total_count: number;
  truncated: boolean;
  limit: number;
  sort: string;
}

export interface Total {
  count: number;
  counted: number;
  allocated: number;
  apparent: number;
}

export interface TopResponse extends Header {
  path: string;
  kind: string;
  min_size: number | null;
  older_than: number | null;
  sort: string;
  entries: Entry[];
  total: Total;
  total_count: number;
  total_allocated: number;
  truncated: boolean;
  limit: number;
}

export interface FindEntry extends Entry {
  nested: boolean;
}

export interface FindResponse extends Header {
  path: string;
  kind: string;
  sort: string;
  entries: FindEntry[];
  total: Total;
  total_count: number;
  total_allocated: number;
  truncated: boolean;
  limit: number;
}

export type DiffStatus = "new" | "gone" | "changed" | "same";

export interface DiffEntry {
  path: string;
  path_raw?: string;
  name: string;
  kind: Kind;
  status: DiffStatus;
  allocated: number;
  allocated_prev: number;
  delta: number;
  items: number;
  items_prev: number;
  mtime: number;
  newest_mtime: number;
}

export interface Refreshed {
  path: string;
  scanned_at: number;
  generation: number;
}

export interface DiffResponse extends Header {
  path: string;
  prev_scanned_at: number;
  depth: number;
  kind: string;
  partial: boolean;
  refreshed: Refreshed[];
  roots_changed: boolean;
  delta: number;
  entries: DiffEntry[];
  total_count: number;
  truncated: boolean;
  limit: number;
}

export interface RootOut {
  path: string;
  fs_type: string;
  allocated: number;
  items: number;
  fs_used: number;
  fs_total: number;
  fs_avail: number;
}

export interface Scanning {
  op: string;
  path: string;
  started_at: number;
  elapsed_seconds: number;
  pid: number;
}

export interface Progress {
  op: string;
  path: string;
  stage: string;
  started_at: number;
  updated_at: number;
  elapsed_seconds: number;
  entries_seen: number;
  entries_expected: number | null;
  percent: number | null;
}

export interface LastError {
  ts: number;
  op: string;
  message: string;
}

export interface Status {
  host: string;
  os: string;
  version: string;
  snapshot_dir: string;
  generation: number | null;
  prev_generation: number | null;
  schema_version: number | null;
  snapshot_id: string | null;
  scanned_at: number | null;
  age_seconds: number | null;
  entries: number | null;
  prev: { generation: number; scanned_at: number } | null;
  refreshed: Refreshed[];
  roots: RootOut[];
  fs: { path: string; fs_type: string; total: number; used: number; avail: number | null }[];
  incomplete: boolean;
  unknown_subtrees: string[];
  last_error: LastError | null;
  last_success_ts: number | null;
  warnings: string[];
  state_error?: string;
  scanning: Scanning | null;
  progress?: Progress;
}

export interface ErrorDetail {
  code: string;
  exit: number;
  message: string;
  fix: string | null;
  docs: string;
}
