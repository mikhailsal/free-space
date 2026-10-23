// The only module that knows the transport (plan line 151).

import type { ErrorDetail } from "./types";

export type FailureKind = "session" | "failure" | "api" | "aborted";

export class ApiError extends Error {
  constructor(
    public kind: FailureKind,
    public status: number,
    public detail?: ErrorDetail,
  ) {
    super(detail?.message ?? kind);
  }

  get code(): string | undefined {
    return this.detail?.code;
  }
}

export type Params = Record<string, string | number | boolean | undefined | null>;

const TOKEN_KEY = "freespace.token";
let token = "";

export function setToken(t: string): void {
  token = t;
  try {
    sessionStorage.setItem(TOKEN_KEY, t);
  } catch {
    // private mode: the hash still carries it
  }
}

export function storedToken(): string {
  try {
    return sessionStorage.getItem(TOKEN_KEY) ?? "";
  } catch {
    return "";
  }
}

export function hasToken(): boolean {
  return token !== "";
}

export function query(params: Params): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === "" || v === false) continue;
    q.set(k, v === true ? "true" : String(v));
  }
  const s = q.toString();
  return s ? `?${s}` : "";
}

export async function get<T>(endpoint: string, params: Params = {}, signal?: AbortSignal): Promise<T> {
  let r: Response;
  try {
    r = await fetch(`/api/${endpoint}${query(params)}`, {
      headers: { "X-Freespace-Token": token },
      cache: "no-store",
      signal,
    });
  } catch {
    // refused connection = the server stopped (D7 "Session ended")
    throw new ApiError(signal?.aborted ? "aborted" : "session", 0);
  }
  if (r.ok) return (await r.json()) as T;
  if ((r.headers.get("content-type") ?? "").includes("application/json")) {
    const body = (await r.json()) as { error?: ErrorDetail };
    if (body.error) throw new ApiError("api", r.status, body.error);
  }
  if (r.status === 403) throw new ApiError("session", 403);
  throw new ApiError("failure", r.status);
}
