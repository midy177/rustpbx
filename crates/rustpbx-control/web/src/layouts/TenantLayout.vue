<script setup lang="ts">
import { computed } from "vue";
import AppShell, { type NavItem } from "@/components/AppShell.vue";
import { useAuthStore } from "@/stores/auth";
import { UserCircle } from "lucide-vue-next";

const auth = useAuthStore();
const scopeLabel = computed(() => `Tenant #${auth.activeTenantId ?? "?"}`);

const nav: NavItem[] = [
  { to: "/me/profile", labelKey: "nav.profile", icon: UserCircle },
];
</script>

<template>
  <AppShell
    area-title-key="nav.tenant"
    :nav="nav"
    :scope-label="scopeLabel"
    exit-to="/admin/tenants"
  />
</template>
