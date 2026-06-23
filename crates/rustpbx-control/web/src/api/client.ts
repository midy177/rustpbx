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
    // An expired/invalid session on any authenticated call → notify the app so
    // it can reset state and bounce to login. The login call handles its own
    // 401 (bad credentials) inline, so it's excluded.
    if (res.status === 401 && path !== "/auth/login") {
      window.dispatchEvent(new Event("rustpbx:unauthorized"));
    }
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
  role: string; // superadmin | tenant_admin | tenant_user
  tenant_id: number | null;
  permissions: string[];
}

export interface Tenant {
  id: number;
  name: string;
  status: string;
  max_concurrent_calls: number | null;
  max_trunks: number | null;
  max_dids: number | null;
  storage_prefix: string | null;
  custom_domain: string | null;
  custom_domain_enabled: boolean;
  default_domain: string | null;
  active_domain: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateTenant {
  name: string;
  max_concurrent_calls?: number | null;
  max_trunks?: number | null;
  max_dids?: number | null;
  storage_prefix?: string | null;
  custom_domain?: string | null;
  admin_username?: string | null;
  admin_password?: string | null;
}

export interface UpdateTenant {
  name?: string;
  status?: string;
  max_concurrent_calls?: number | null;
  max_trunks?: number | null;
  max_dids?: number | null;
  storage_prefix?: string | null;
}

export interface PlatformSettings {
  base_domain: string;
  stun_servers: string[];
}

// ── Tenant IAM users ─────────────────────────────────────────────────────────

export interface TenantUser {
  id: number;
  tenant_id: number;
  username: string;
  display_name: string | null;
  role: string; // admin | user
  permissions: string[];
  status: string; // active | suspended
  created_at: string;
  updated_at: string;
  last_login_at: string | null;
}

export interface CreateTenantUser {
  username: string;
  password: string;
  display_name?: string | null;
  role?: string;
  permissions?: string[];
}

export interface UpdateTenantUser {
  display_name?: string | null;
  password?: string | null;
  role?: string;
  permissions?: string[];
  status?: string;
}

export interface UpdateDomain {
  custom_domain: string | null;
  custom_domain_enabled: boolean;
}

// ── Extensions ───────────────────────────────────────────────────────────────

export interface Extension {
  id: number;
  extension: string;
  tenant_id: number | null;
  display_name: string | null;
  email: string | null;
  status: string | null;
  login_disabled: boolean;
  voicemail_disabled: boolean;
  allow_guest_calls: boolean;
  call_forwarding_mode: string | null;
  call_forwarding_destination: string | null;
  call_forwarding_timeout: number | null;
}

export interface ExtensionInput {
  extension: string;
  display_name?: string | null;
  email?: string | null;
  status?: string | null;
  login_disabled?: boolean;
  voicemail_disabled?: boolean;
  allow_guest_calls?: boolean;
  sip_password?: string | null;
  call_forwarding_mode?: string | null;
  call_forwarding_destination?: string | null;
  call_forwarding_timeout?: number | null;
}

export interface TrunkInput {
  name: string;
  display_name?: string | null;
  carrier?: string | null;
  direction?: string;
  sip_server?: string | null;
  sip_transport?: string;
  outbound_proxy?: string | null;
  auth_username?: string | null;
  auth_password?: string | null;
  max_cps?: number | null;
  max_concurrent?: number | null;
  allowed_ips?: string[];
  did_numbers?: string[];
  incoming_from_user_prefix?: string | null;
  incoming_to_user_prefix?: string | null;
  is_active?: boolean;
  register_enabled?: boolean;
  register_expires?: number | null;
  rewrite_hostport?: boolean;
}

export interface RouteInput {
  name: string;
  description?: string | null;
  direction?: string;
  priority?: number;
  is_active?: boolean;
  selection_strategy?: string;
  hash_key?: string | null;
  source_pattern?: string | null;
  destination_pattern?: string | null;
  target_trunks?: string[];
}

export interface Did {
  id: number;
  number: string;
  tenant_id: number | null;
  trunk_id: number | null;
  status: string; // available | assigned | reserved | porting
  country: string | null;
  city: string | null;
  monthly_cost: number | null;
  created_at: string;
  updated_at: string;
}

export interface CreateDid {
  number: string;
  tenant_id?: number | null;
  status?: string;
  country?: string | null;
  city?: string | null;
  monthly_cost?: number | null;
}

export interface UpdateDid {
  tenant_id?: number | null;
  trunk_id?: number | null;
  status?: string;
  country?: string | null;
  city?: string | null;
  monthly_cost?: number | null;
  unassign?: boolean;
}

export interface TenantStats {
  trunks: number;
  extensions: number;
  dids: number;
  recent_calls: number;
}

export interface AclRule {
  id: number;
  tenant_id: number | null;
  action: string; // allow | deny
  target: string; // CIDR or "all"
  priority: number;
  is_active: boolean;
}

export interface AclInput {
  action: string;
  target: string;
  priority?: number;
  is_active?: boolean;
}

export interface CallRecord {
  id: number;
  call_id: string;
  tenant_id: number | null;
  direction: string;
  status: string;
  from_number: string | null;
  to_number: string | null;
  started_at: string | null;
  ended_at: string | null;
  duration_secs: number;
  recording_url: string | null;
}

/** Permission catalogue values (mirror `auth::permissions`). */
export const ALL_PERMISSIONS = [
  "trunks:read",
  "trunks:write",
  "routing:read",
  "routing:write",
  "extensions:read",
  "extensions:write",
  "cdr:read",
  "dids:read",
  "dids:write",
  "acl:read",
  "acl:write",
  "users:read",
  "users:write",
  "domain:read",
  "domain:write",
] as const;

export interface Stats {
  tenants: number;
  workers_total: number;
  workers_healthy: number;
  active_calls: number;
}

export interface Trunk {
  id: number;
  name: string;
  dest: string | null;
  transport: string;
  direction: string;
  has_auth: boolean;
  register_enabled: boolean;
  is_active: boolean;
  did_numbers: string[];
  allowed_ips: string[];
  max_concurrent: number | null;
  tenant_id: number | null;
}

export interface Route {
  id: number;
  name: string;
  description: string | null;
  priority: number;
  direction: string;
  source_pattern: string | null;
  destination_pattern: string | null;
  target_trunks: string[];
  is_active: boolean;
  tenant_id: number | null;
}

export interface Edge {
  edge_id: string;
  public_ip: string;
  sip_addr: string;
  transports: string[];
  region: string;
  version: string;
  active_calls: number;
  nat_type: string;
  registered_at: string;
  last_heartbeat_secs_ago: number;
  healthy: boolean;
}

export interface Worker {
  worker_id: string;
  sip_addr: string;
  rtp_external_ip: string;
  active_calls: number;
  max_concurrent: number;
  available_capacity: number;
  cpu_usage: number;
  nat_type: string;
  registered_at: string;
  last_heartbeat_secs_ago: number;
  healthy: boolean;
  draining: boolean;
}
