<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { api, type AuditEntry } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const route = useRoute();
const auth = useAuthStore();

const entries = ref<AuditEntry[]>([]);
const loading = ref(true);
const error = ref("");
const actionFilter = ref("");
const typeFilter = ref("");

// Superadmin area shows all entries; tenant area is scoped to the active
// tenant (the backend enforces this too).
const isPlatform = computed(() => route.name === "admin-audit");

function actionVariant(a: string) {
  if (a === "delete") return "warning" as const;
  if (a === "create") return "success" as const;
  return "muted" as const;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const p = new URLSearchParams();
    if (!isPlatform.value && auth.activeTenantId) {
      p.set("tenant_id", String(auth.activeTenantId));
    }
    if (actionFilter.value) p.set("action", actionFilter.value);
    if (typeFilter.value) p.set("target_type", typeFilter.value);
    p.set("limit", "200");
    entries.value = await api.get<AuditEntry[]>(`/audit?${p.toString()}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}

// Re-fetch when switching between platform and tenant areas.
watch(isPlatform, load);
onMounted(load);
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("auditPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("auditPage.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <Card class="p-4">
      <form class="flex flex-wrap items-end gap-3" @submit.prevent="load">
        <select
          v-model="actionFilter"
          class="h-9 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm"
        >
          <option value="">{{ t("auditPage.allActions") }}</option>
          <option value="create">create</option>
          <option value="update">update</option>
          <option value="delete">delete</option>
        </select>
        <select
          v-model="typeFilter"
          class="h-9 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm"
        >
          <option value="">{{ t("auditPage.allTypes") }}</option>
          <option value="tenant">tenant</option>
          <option value="tenant_user">user</option>
          <option value="trunk">trunk</option>
          <option value="route">route</option>
          <option value="extension">extension</option>
          <option value="acl">acl</option>
          <option value="did">did</option>
          <option value="domain">domain</option>
        </select>
        <Button type="submit">{{ t("common.refresh") }}</Button>
      </form>
    </Card>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("auditPage.time") }}</TableHead>
            <TableHead>{{ t("auditPage.actor") }}</TableHead>
            <TableHead>{{ t("auditPage.action") }}</TableHead>
            <TableHead>{{ t("auditPage.target") }}</TableHead>
            <TableHead>{{ t("auditPage.summary") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="5">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="entries.length === 0" :colspan="5">{{ t("auditPage.noEntries") }}</TableEmpty>
          <TableRow v-for="e in entries" :key="e.id">
            <TableCell class="text-muted-foreground text-xs">
              {{ formatDate(e.created_at) }}
            </TableCell>
            <TableCell>
              <div class="font-medium">{{ e.actor_username }}</div>
              <div class="text-muted-foreground text-xs">{{ e.actor_role }}</div>
            </TableCell>
            <TableCell><Badge :variant="actionVariant(e.action)">{{ e.action }}</Badge></TableCell>
            <TableCell class="font-mono text-xs">{{ e.target_type }}<span v-if="e.target_id" class="text-muted-foreground">#{{ e.target_id }}</span></TableCell>
            <TableCell class="text-sm">{{ e.summary }}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
