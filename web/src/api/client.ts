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
};
