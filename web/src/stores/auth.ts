import { defineStore } from "pinia";
import { api, type SessionUser } from "@/api/client";

interface AuthState {
  user: SessionUser | null;
  loading: boolean;
}

export const useAuthStore = defineStore("auth", {
  state: (): AuthState => ({
    user: null,
    loading: false,
  }),
  getters: {
    isAuthenticated: (state) => Boolean(state.user),
    tenantName: (state) => state.user?.tenant?.name ?? "Default",
  },
  actions: {
    async restore() {
      this.loading = true;
      try {
        this.user = await api.session();
      } finally {
        this.loading = false;
      }
    },
    async login(username: string, password: string, tenant?: string) {
      this.user = await api.login({ username, password, tenant });
    },
    clear() {
      this.user = null;
    },
  },
});
