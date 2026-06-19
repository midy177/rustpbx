// Thin fetch wrapper for the Control Plane HTTP API.
// Token is read from the auth store's localStorage key on each call so the
// client stays decoupled from Pinia (avoids a circular import).

const TOKEN_KEY = "rustpbx.control.token";

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}
export function setToken(token: string | null) {
  if (token) localStorage.setItem(TOKEN_KEY, token);
  else localStorage.removeItem(TOKEN_KEY);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers["Authorization"] = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";

  const res = await fetch(`/api${path}`, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });

  if (res.status === 204) return undefined as T;

  const text = await res.text();
  const data = text ? JSON.parse(text) : null;

  if (!res.ok) {
    const msg = (data && (data.error as string)) || res.statusText;
    throw new ApiError(res.status, msg);
  }
  return data as T;
}

export const api = {
  get: <T>(p: string) => request<T>("GET", p),
  post: <T>(p: string, b?: unknown) => request<T>("POST", p, b),
  put: <T>(p: string, b?: unknown) => request<T>("PUT", p, b),
  del: <T>(p: string) => request<T>("DELETE", p),
};

// ── Domain types (mirror the Rust API responses) ─────────────────────────────

export interface UserInfo {
  username: string;
  role: string;
  tenant_id: number | null;
}

export interface Tenant {
  id: number;
  name: string;
  status: string;
  max_concurrent_calls: number | null;
  max_trunks: number | null;
  max_dids: number | null;
  storage_prefix: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateTenant {
  name: string;
  max_concurrent_calls?: number | null;
  max_trunks?: number | null;
  max_dids?: number | null;
  storage_prefix?: string | null;
}

export interface UpdateTenant {
  name?: string;
  status?: string;
  max_concurrent_calls?: number | null;
  max_trunks?: number | null;
  max_dids?: number | null;
  storage_prefix?: string | null;
}

export interface Stats {
  tenants: number;
  workers_total: number;
  workers_healthy: number;
  active_calls: number;
}

export interface Worker {
  worker_id: string;
  sip_addr: string;
  rtp_external_ip: string;
  active_calls: number;
  max_concurrent: number;
  available_capacity: number;
  cpu_usage: number;
  registered_at: string;
  last_heartbeat_secs_ago: number;
  healthy: boolean;
  draining: boolean;
}
