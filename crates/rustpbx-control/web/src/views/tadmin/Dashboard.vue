<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Tenant } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

const { t } = useI18n();
const auth = useAuthStore();
const tenant = ref<Tenant | null>(null);
const loading = ref(true);

onMounted(async () => {
  if (!auth.activeTenantId) return;
  try {
    tenant.value = await api.get<Tenant>(`/tenants/${auth.activeTenantId}`);
  } finally {
    loading.value = false;
  }
});

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

    <Card v-if="tenant">
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          {{ tenant.name }}
          <Badge variant="success">{{ tenant.status }}</Badge>
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
