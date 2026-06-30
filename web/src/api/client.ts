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
  sipTrunks() {
    return request<SipTrunkSummary[]>("/cloudpbx/sip-trunks");
  },
};
