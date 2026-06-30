import { createRouter, createWebHistory } from "vue-router";
import Dashboard from "@/views/Dashboard.vue";
import Login from "@/views/Login.vue";
import { useAuthStore } from "@/stores/auth";

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: "/login", name: "login", component: Login },
    { path: "/", name: "dashboard", component: Dashboard, meta: { requiresAuth: true } },
  ],
});

router.beforeEach(async (to) => {
  const auth = useAuthStore();
  if (!to.meta.requiresAuth || auth.isAuthenticated) {
    return true;
  }
  try {
    await auth.restore();
    return true;
  } catch {
    return { name: "login", query: { redirect: to.fullPath } };
  }
});

export default router;
