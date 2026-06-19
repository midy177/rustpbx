<script setup lang="ts">
import AppShell, { type NavItem } from "@/components/AppShell.vue";
import { LayoutDashboard, Building2, Server } from "lucide-vue-next";

const nav: NavItem[] = [
  { to: "/admin/dashboard", labelKey: "nav.dashboard", icon: LayoutDashboard },
  { to: "/admin/tenants", labelKey: "nav.tenants", icon: Building2 },
  { to: "/admin/workers", labelKey: "nav.workers", icon: Server },
];
</script>

<template>
  <AppShell area-title-key="nav.superAdmin" :nav="nav" />
</template>
