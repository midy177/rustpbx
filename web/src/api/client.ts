export interface TenantSummary {
  id: string;
  name: string;
  status: "active" | "suspended" | "disabled";
  domain?: string;
}

export interface SessionUser {
  id: number;
  username: string;
  email?: string;
  role: "platform_admin" | "tenant_admin" | "tenant_user";
  tenant?: TenantSummary;
}

export interface ExtensionSummary {
  id: number;
  tenant_id?: number | null;
  extension: string;
  display_name?: string | null;
  email?: string | null;
  status?: string | null;
  login_disabled: boolean;
  voicemail_disabled: boolean;
  allow_guest_calls: boolean;
}

export interface CreateExtensionRequest {
  extension: string;
  display_name?: string | null;
  email?: string | null;
  status?: string | null;
  login_disabled?: boolean;
  voicemail_disabled?: boolean;
  allow_guest_calls?: boolean;
  notes?: string | null;
}

export interface UpdateExtensionRequest {
  extension?: string;
  display_name?: string | null;
  email?: string | null;
  status?: string | null;
  login_disabled?: boolean;
  voicemail_disabled?: boolean;
  allow_guest_calls?: boolean;
  notes?: string | null;
}

export interface SipTrunkSummary {
  id: number;
  tenant_id?: number | null;
  name: string;
  display_name?: string | null;
  carrier?: string | null;
  status: string;
  direction: string;
  sip_server?: string | null;
  sip_transport: string;
  is_active: boolean;
  register_enabled: boolean;
}

export interface CreateSipTrunkRequest {
  name: string;
  display_name?: string | null;
  carrier?: string | null;
  description?: string | null;
  status?: string;
  direction?: string;
  sip_server?: string | null;
  sip_transport?: string;
  outbound_proxy?: string | null;
  auth_username?: string | null;
  auth_password?: string | null;
  is_active?: boolean;
  register_enabled?: boolean;
}

export interface UpdateSipTrunkRequest {
  name?: string;
  display_name?: string | null;
  carrier?: string | null;
  description?: string | null;
  status?: string;
  direction?: string;
  sip_server?: string | null;
  sip_transport?: string;
  outbound_proxy?: string | null;
  auth_username?: string | null;
  auth_password?: string | null;
  is_active?: boolean;
  register_enabled?: boolean;
}

export interface RouteSummary {
  id: number;
  tenant_id?: number | null;
  name: string;
  description?: string | null;
  direction: string;
  priority: number;
  is_active: boolean;
  selection_strategy: string;
  source_trunk_id?: number | null;
  default_trunk_id?: number | null;
  source_pattern?: string | null;
  destination_pattern?: string | null;
  owner?: string | null;
}

export interface CreateRouteRequest {
  name: string;
  description?: string | null;
  direction?: string;
  priority?: number;
  is_active?: boolean;
  selection_strategy?: string;
  source_trunk_id?: number | null;
  default_trunk_id?: number | null;
  source_pattern?: string | null;
  destination_pattern?: string | null;
  owner?: string | null;
}

export interface UpdateRouteRequest {
  name?: string;
  description?: string | null;
  direction?: string;
  priority?: number;
  is_active?: boolean;
  selection_strategy?: string;
  source_trunk_id?: number | null;
  default_trunk_id?: number | null;
  source_pattern?: string | null;
  destination_pattern?: string | null;
  owner?: string | null;
}

export interface CallRecordSummary {
  id: number;
  tenant_id?: number | null;
  call_id: string;
  display_id?: string | null;
  direction: string;
  status: string;
  started_at: string;
  ended_at?: string | null;
  duration_secs: number;
  from_number?: string | null;
  to_number?: string | null;
  caller_name?: string | null;
  agent_name?: string | null;
  queue?: string | null;
  extension_id?: number | null;
  sip_trunk_id?: number | null;
  route_id?: number | null;
  has_transcript: boolean;
  transcript_status: string;
  recording_duration_secs?: number | null;
}

export interface UserSummary {
  id: number;
  tenant_id?: number | null;
  email: string;
  username: string;
  last_login_at?: string | null;
  is_active: boolean;
  is_staff: boolean;
  is_superuser: boolean;
  mfa_enabled: boolean;
  auth_source: string;
}

export interface CreateUserRequest {
  username: string;
  email: string;
  password: string;
  is_active?: boolean;
  is_staff?: boolean;
  is_superuser?: boolean;
}

export interface UpdateUserRequest {
  username?: string;
  email?: string;
  password?: string;
  is_active?: boolean;
  is_staff?: boolean;
  is_superuser?: boolean;
}

export interface LoginRequest {
  username: string;
  password: string;
  tenant?: string;
}

const API_BASE = "/api";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    credentials: "include",
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
    ...init,
  });
  if (response.status === 401) {
    window.dispatchEvent(new CustomEvent("cloudpbx:unauthorized"));
  }
  if (!response.ok) {
    throw new Error(`API ${response.status}: ${response.statusText}`);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return response.json() as Promise<T>;
}

export const api = {
  login(payload: LoginRequest) {
    return request<SessionUser>("/auth/login", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  session() {
    return request<SessionUser>("/auth/session");
  },
  tenants() {
    return request<TenantSummary[]>("/tenants");
  },
  extensions() {
    return request<ExtensionSummary[]>("/cloudpbx/extensions");
  },
  createExtension(payload: CreateExtensionRequest) {
    return request<ExtensionSummary>("/cloudpbx/extensions", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  updateExtension(id: number, payload: UpdateExtensionRequest) {
    return request<ExtensionSummary>(`/cloudpbx/extensions/${id}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },
  deleteExtension(id: number) {
    return request<void>(`/cloudpbx/extensions/${id}`, {
      method: "DELETE",
    });
  },
  sipTrunks() {
    return request<SipTrunkSummary[]>("/cloudpbx/sip-trunks");
  },
  createSipTrunk(payload: CreateSipTrunkRequest) {
    return request<SipTrunkSummary>("/cloudpbx/sip-trunks", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  updateSipTrunk(id: number, payload: UpdateSipTrunkRequest) {
    return request<SipTrunkSummary>(`/cloudpbx/sip-trunks/${id}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },
  deleteSipTrunk(id: number) {
    return request<void>(`/cloudpbx/sip-trunks/${id}`, {
      method: "DELETE",
    });
  },
  routes() {
    return request<RouteSummary[]>("/cloudpbx/routes");
  },
  createRoute(payload: CreateRouteRequest) {
    return request<RouteSummary>("/cloudpbx/routes", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  updateRoute(id: number, payload: UpdateRouteRequest) {
    return request<RouteSummary>(`/cloudpbx/routes/${id}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },
  deleteRoute(id: number) {
    return request<void>(`/cloudpbx/routes/${id}`, {
      method: "DELETE",
    });
  },
  callRecords() {
    return request<CallRecordSummary[]>("/cloudpbx/call-records");
  },
  users() {
    return request<UserSummary[]>("/cloudpbx/users");
  },
  createUser(payload: CreateUserRequest) {
    return request<UserSummary>("/cloudpbx/users", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
  updateUser(id: number, payload: UpdateUserRequest) {
    return request<UserSummary>(`/cloudpbx/users/${id}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },
  deleteUser(id: number) {
    return request<void>(`/cloudpbx/users/${id}`, {
      method: "DELETE",
    });
  },
};
