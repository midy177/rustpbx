import { createRouter, createWebHashHistory } from "vue-router";
import { useAuthStore } from "@/stores/auth";

const routes = [
  {
    path: "/login",
    name: "login",
    component: () => import("@/views/Login.vue"),
    meta: { layout: "auth", public: true },
  },
  // ── Super-admin (platform) area ───────────────────────────────────────────
  {
    path: "/admin",
    component: () => import("@/layouts/SuperAdminLayout.vue"),
    meta: { area: "admin" },
    children: [
      { path: "", redirect: "/admin/dashboard" },
      { path: "dashboard", name: "admin-dashboard", component: () => import("@/views/admin/Dashboard.vue") },
      { path: "tenants", name: "admin-tenants", component: () => import("@/views/admin/Tenants.vue") },
      { path: "workers", name: "admin-workers", component: () => import("@/views/admin/Workers.vue") },
    ],
  },
  // ── Tenant-admin area (scoped to the active tenant) ────────────────────────
  {
    path: "/t/admin",
    component: () => import("@/layouts/TenantAdminLayout.vue"),
    meta: { area: "tenant-admin" },
    children: [
      { path: "", redirect: "/t/admin/dashboard" },
      { path: "dashboard", name: "tadmin-dashboard", component: () => import("@/views/tadmin/Dashboard.vue") },
      { path: "trunks", name: "tadmin-trunks", component: () => import("@/views/tadmin/Trunks.vue") },
      { path: "routing", name: "tadmin-routing", component: () => import("@/views/tadmin/Routing.vue") },
      { path: "extensions", name: "tadmin-extensions", component: () => import("@/views/tadmin/Extensions.vue") },
      { path: "call-records", name: "tadmin-cdr", component: () => import("@/views/tadmin/CallRecords.vue") },
    ],
  },
  // ── Tenant end-user area ───────────────────────────────────────────────────
  {
    path: "/me",
    component: () => import("@/layouts/TenantLayout.vue"),
    meta: { area: "tenant" },
    children: [
      { path: "", redirect: "/me/profile" },
      { path: "profile", name: "tenant-profile", component: () => import("@/views/tenant/Profile.vue") },
    ],
  },
  { path: "/", redirect: "/admin/dashboard" },
  { path: "/:pathMatch(.*)*", redirect: "/admin/dashboard" },
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

router.beforeEach((to) => {
  const auth = useAuthStore();
  if (!to.meta.public && !auth.isAuthenticated) {
    return { name: "login", query: { redirect: to.fullPath } };
  }
  if (to.name === "login" && auth.isAuthenticated) {
    return { path: "/admin/dashboard" };
  }
  // Tenant-admin / tenant areas require an active tenant to be selected.
  if ((to.meta.area === "tenant-admin" || to.meta.area === "tenant") && !auth.activeTenantId) {
    return { path: "/admin/tenants" };
  }
  return true;
});
