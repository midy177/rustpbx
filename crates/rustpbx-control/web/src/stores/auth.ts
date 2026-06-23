import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { api, getToken, setToken, type UserInfo } from "@/api/client";

const USER_KEY = "rustpbx.control.user";

function loadUser(): UserInfo | null {
  const raw = localStorage.getItem(USER_KEY);
  return raw ? (JSON.parse(raw) as UserInfo) : null;
}

export const useAuthStore = defineStore("auth", () => {
  const user = ref<UserInfo | null>(loadUser());
  const token = ref<string | null>(getToken());

  const isAuthenticated = computed(() => !!token.value && !!user.value);
  const isSuperAdmin = computed(() => user.value?.role === "superadmin");
  const isTenantAdmin = computed(() => user.value?.role === "tenant_admin");
  const isTenantUser = computed(() => user.value?.role === "tenant_user");
  /** Any tenant-scoped principal (admin or plain user). */
  const isTenantScoped = computed(() => isTenantAdmin.value || isTenantUser.value);

  /**
   * The tenant id the UI is currently scoped to. For a tenant principal this is
   * their own tenant; for a super-admin it's the tenant they "entered".
   */
  const activeTenantId = ref<number | null>(
    Number(localStorage.getItem("rustpbx.control.activeTenant")) || null,
  );

  function setActiveTenant(id: number | null) {
    activeTenantId.value = id;
    if (id) localStorage.setItem("rustpbx.control.activeTenant", String(id));
    else localStorage.removeItem("rustpbx.control.activeTenant");
  }

  /** Whether the current principal holds a permission (admins always do). */
  function can(perm: string): boolean {
    const u = user.value;
    if (!u) return false;
    if (u.role === "superadmin" || u.role === "tenant_admin") return true;
    return (u.permissions ?? []).includes(perm);
  }

  /** Landing route after login, by role. */
  function homeRoute(): string {
    if (isSuperAdmin.value) return "/admin/dashboard";
    return "/t/admin/dashboard";
  }

  async function login(username: string, password: string, domain?: string) {
    const resp = await api.post<{ token: string; user: UserInfo }>("/auth/login", {
      username,
      password,
      domain: domain?.trim() || undefined,
    });
    token.value = resp.token;
    user.value = resp.user;
    setToken(resp.token);
    localStorage.setItem(USER_KEY, JSON.stringify(resp.user));
    // Tenant principals are pinned to their own tenant.
    if (resp.user.tenant_id) setActiveTenant(resp.user.tenant_id);
  }

  async function logout() {
    try {
      await api.post("/auth/logout");
    } catch {
      /* ignore */
    }
    token.value = null;
    user.value = null;
    setToken(null);
    localStorage.removeItem(USER_KEY);
    setActiveTenant(null);
  }

  return {
    user,
    token,
    isAuthenticated,
    isSuperAdmin,
    isTenantAdmin,
    isTenantUser,
    isTenantScoped,
    activeTenantId,
    setActiveTenant,
    can,
    homeRoute,
    login,
    logout,
  };
});
