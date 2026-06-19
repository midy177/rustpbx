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

  /** The tenant id the UI is currently scoped to (super-admin "enter tenant"). */
  const activeTenantId = ref<number | null>(
    Number(localStorage.getItem("rustpbx.control.activeTenant")) || null,
  );

  function setActiveTenant(id: number | null) {
    activeTenantId.value = id;
    if (id) localStorage.setItem("rustpbx.control.activeTenant", String(id));
    else localStorage.removeItem("rustpbx.control.activeTenant");
  }

  async function login(username: string, password: string) {
    const resp = await api.post<{ token: string; user: UserInfo }>("/auth/login", {
      username,
      password,
    });
    token.value = resp.token;
    user.value = resp.user;
    setToken(resp.token);
    localStorage.setItem(USER_KEY, JSON.stringify(resp.user));
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
    activeTenantId,
    setActiveTenant,
    login,
    logout,
  };
});
