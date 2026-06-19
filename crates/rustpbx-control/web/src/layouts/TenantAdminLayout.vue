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
} from "lucide-vue-next";

const auth = useAuthStore();
const scopeLabel = computed(() => `Tenant #${auth.activeTenantId ?? "?"}`);

const nav: NavItem[] = [
  { to: "/t/admin/dashboard", labelKey: "nav.dashboard", icon: LayoutDashboard },
  { to: "/t/admin/trunks", labelKey: "nav.trunks", icon: Cable },
  { to: "/t/admin/routing", labelKey: "nav.routing", icon: Route },
  { to: "/t/admin/extensions", labelKey: "nav.extensions", icon: PhoneCall },
  { to: "/t/admin/call-records", labelKey: "nav.callRecords", icon: ScrollText },
];
</script>

<template>
  <AppShell
    area-title-key="nav.tenantAdmin"
    :nav="nav"
    :scope-label="scopeLabel"
    exit-to="/admin/tenants"
  />
</template>
