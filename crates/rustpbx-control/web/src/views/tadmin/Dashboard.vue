<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Tenant, type TenantStats } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Cable, PhoneCall, Hash, ScrollText } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const tenant = ref<Tenant | null>(null);
const stats = ref<TenantStats | null>(null);
const loading = ref(true);

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

onMounted(async () => {
  if (!auth.activeTenantId) return;
  try {
    const [tn, st] = await Promise.all([
      api.get<Tenant>(`/tenants/${auth.activeTenantId}`).catch(() => null),
      api.get<TenantStats>(`/tenant-stats${scope()}`).catch(() => null),
    ]);
    tenant.value = tn;
    stats.value = st;
  } finally {
    loading.value = false;
  }
});

const statCards = () => [
  { key: "tdashboard.trunks", v: stats.value?.trunks, icon: Cable },
  { key: "tdashboard.extensions", v: stats.value?.extensions, icon: PhoneCall },
  { key: "tdashboard.dids", v: stats.value?.dids, icon: Hash },
  { key: "tdashboard.recentCalls", v: stats.value?.recent_calls, icon: ScrollText },
];

const quota = () => [
  { key: "tenants.maxConcurrentCalls", v: tenant.value?.max_concurrent_calls },
  { key: "tenants.maxTrunks", v: tenant.value?.max_trunks },
  { key: "tenants.maxDids", v: tenant.value?.max_dids },
];
</script>

<template>
  <div class="space-y-6">
    <div>
      <h2 class="text-2xl font-bold tracking-tight">{{ t("dashboard.title") }}</h2>
      <p class="text-sm text-muted-foreground">{{ t("tenantArea.scopedTo") }}: {{ tenant?.name ?? "…" }}</p>
    </div>

    <!-- Live counts -->
    <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <Card v-for="c in statCards()" :key="c.key">
        <CardContent class="flex items-center gap-4 pt-6">
          <div class="rounded-lg bg-muted p-3">
            <component :is="c.icon" class="size-5 text-muted-foreground" />
          </div>
          <div>
            <div class="text-2xl font-bold">{{ c.v ?? "—" }}</div>
            <div class="text-xs text-muted-foreground">{{ t(c.key) }}</div>
          </div>
        </CardContent>
      </Card>
    </div>

    <!-- Domain + quota -->
    <Card v-if="tenant">
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          {{ tenant.name }}
          <Badge variant="success">{{ tenant.status }}</Badge>
          <Badge v-if="tenant.active_domain" variant="secondary" class="font-mono">{{ tenant.active_domain }}</Badge>
        </CardTitle>
        <CardDescription>
          {{ t("common.id") }}: {{ tenant.id }} · {{ t("common.createdAt") }}: {{ formatDate(tenant.created_at) }}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div class="grid gap-4 sm:grid-cols-3">
          <div v-for="q in quota()" :key="q.key" class="rounded-lg border p-4">
            <div class="text-xs text-muted-foreground">{{ t(q.key) }}</div>
            <div class="mt-1 text-2xl font-bold">{{ q.v ?? t("common.unlimited") }}</div>
          </div>
        </div>
      </CardContent>
    </Card>
    <p v-else-if="!loading" class="text-sm text-muted-foreground">{{ t("common.empty") }}</p>
  </div>
</template>
