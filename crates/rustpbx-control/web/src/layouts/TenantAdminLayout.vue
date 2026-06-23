<script setup lang="ts">
import { computed } from "vue";
import AppShell, { type NavItem } from "@/components/AppShell.vue";
import { useAuthStore } from "@/stores/auth";
import {
  LayoutDashboard,
  Cable,
  Route,
  PhoneCall,
  ScrollText,
  Hash,
  Users,
  Globe,
} from "lucide-vue-next";

const auth = useAuthStore();
const scopeLabel = computed(() => `Tenant #${auth.activeTenantId ?? "?"}`);

// Nav is permission-gated: tenant admins see everything, scoped users only see
// the sections they hold a read permission for.
const allNav: (NavItem & { perm?: string })[] = [
  { to: "/t/admin/dashboard", labelKey: "nav.dashboard", icon: LayoutDashboard },
  { to: "/t/admin/trunks", labelKey: "nav.trunks", icon: Cable, perm: "trunks:read" },
  { to: "/t/admin/routing", labelKey: "nav.routing", icon: Route, perm: "routing:read" },
  { to: "/t/admin/extensions", labelKey: "nav.extensions", icon: PhoneCall, perm: "extensions:read" },
  { to: "/t/admin/dids", labelKey: "nav.dids", icon: Hash, perm: "dids:read" },
  { to: "/t/admin/call-records", labelKey: "nav.callRecords", icon: ScrollText, perm: "cdr:read" },
  { to: "/t/admin/users", labelKey: "nav.users", icon: Users, perm: "users:read" },
  { to: "/t/admin/domain", labelKey: "nav.domain", icon: Globe, perm: "domain:read" },
];

const nav = computed<NavItem[]>(() =>
  allNav.filter((n) => !n.perm || auth.can(n.perm)).map(({ perm: _p, ...item }) => item),
);

// A super-admin who "entered" a tenant can pop back to the tenant list.
const exitTo = computed(() => (auth.isSuperAdmin ? "/admin/tenants" : undefined));
</script>

<template>
  <AppShell
    area-title-key="nav.tenantAdmin"
    :nav="nav"
    :scope-label="scopeLabel"
    :exit-to="exitTo"
  />
</template>
