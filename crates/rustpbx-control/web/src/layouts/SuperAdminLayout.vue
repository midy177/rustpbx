<script setup lang="ts">
import AppShell, { type NavItem } from "@/components/AppShell.vue";
import { LayoutDashboard, Building2, Server, Settings, Hash, Network, History, Boxes } from "lucide-vue-next";

const nav: NavItem[] = [
  { to: "/admin/dashboard", labelKey: "nav.dashboard", icon: LayoutDashboard },
  { to: "/admin/tenants", labelKey: "nav.tenants", icon: Building2 },
  { to: "/admin/dids", labelKey: "nav.dids", icon: Hash },
  { to: "/admin/edges", labelKey: "nav.edges", icon: Network },
  { to: "/admin/workers", labelKey: "nav.workers", icon: Server },
  { to: "/admin/audit", labelKey: "nav.audit", icon: History },
  { to: "/admin/raft", labelKey: "nav.raft", icon: Boxes },
  { to: "/admin/settings", labelKey: "nav.settings", icon: Settings },
];
</script>

<template>
  <AppShell area-title-key="nav.superAdmin" :nav="nav" />
</template>
