<script setup lang="ts">
import { ref, computed, onMounted, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  api, type Stats, type TenantQuota, type Worker, type Edge, type Tenant,
} from "@/api/client";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Building2, Server, PhoneCall, Radio, AlertTriangle, CheckCircle2 } from "lucide-vue-next";

const { t } = useI18n();
const stats = ref<Stats | null>(null);
const quotas = ref<TenantQuota[]>([]);
const workers = ref<Worker[]>([]);
const edges = ref<Edge[]>([]);
const tenants = ref<Tenant[]>([]);
const loading = ref(true);

interface Metric { key: string; value: () => number | string; icon: Component; }
const metrics: Metric[] = [
  { key: "dashboard.tenants", value: () => stats.value?.tenants ?? 0, icon: Building2 },
  {
    key: "dashboard.workers",
    value: () => `${stats.value?.workers_healthy ?? 0} / ${stats.value?.workers_total ?? 0}`,
    icon: Server,
  },
  { key: "dashboard.activeCalls", value: () => stats.value?.active_calls ?? 0, icon: PhoneCall },
  { key: "dashboard.callSlots", value: () => stats.value?.call_slots ?? 0, icon: Radio },
];

const NEAR = 80;
function sat(used: number, max: number | null): number {
  return max && max > 0 ? Math.min(100, (used / max) * 100) : 0;
}

// Needs-attention rollup — aggregates existing endpoints, no new backend.
const nearQuota = computed(() =>
  quotas.value.filter((q) => Math.max(sat(q.trunks.used, q.trunks.max), sat(q.dids.used, q.dids.max), sat(q.concurrent.used, q.concurrent.max)) >= NEAR),
);
const unhealthyWorkers = computed(() => workers.value.filter((w) => !w.healthy && !w.draining));
const offlineEdges = computed(() => edges.value.filter((e) => !e.healthy));
const suspendedTenants = computed(() => tenants.value.filter((tn) => tn.status !== "active"));
const alertCount = computed(() =>
  nearQuota.value.length + unhealthyWorkers.value.length + offlineEdges.value.length + suspendedTenants.value.length,
);

onMounted(async () => {
  try {
    const [st, qu, wk, ed, tn] = await Promise.allSettled([
      api.get<Stats>("/stats"),
      api.get<TenantQuota[]>("/tenant-quotas"),
      api.get<Worker[]>("/workers"),
      api.get<Edge[]>("/edges"),
      api.get<Tenant[]>("/tenants"),
    ]);
    if (st.status === "fulfilled") stats.value = st.value;
    if (qu.status === "fulfilled") quotas.value = qu.value;
    if (wk.status === "fulfilled") workers.value = wk.value;
    if (ed.status === "fulfilled") edges.value = ed.value;
    if (tn.status === "fulfilled") tenants.value = tn.value;
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="space-y-6">
    <div>
      <h2 class="text-2xl font-bold tracking-tight">{{ t("dashboard.title") }}</h2>
      <p class="text-sm text-muted-foreground">{{ t("dashboard.welcome") }}</p>
    </div>

    <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <Card v-for="m in metrics" :key="m.key">
        <CardHeader class="flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle class="text-sm font-medium text-muted-foreground">{{ t(m.key) }}</CardTitle>
          <component :is="m.icon" class="size-4 text-muted-foreground" />
        </CardHeader>
        <CardContent>
          <div class="text-3xl font-bold">{{ loading ? "—" : m.value() }}</div>
        </CardContent>
      </Card>
    </div>

    <!-- Needs attention -->
    <Card>
      <CardHeader class="flex-row items-center justify-between space-y-0">
        <CardTitle class="flex items-center gap-2">
          <AlertTriangle class="size-5 text-destructive" />
          {{ t("dashboard.needsAttention") }}
        </CardTitle>
        <span v-if="alertCount > 0" class="rounded-full bg-destructive px-2 py-0.5 text-xs font-semibold text-destructive-foreground">
          {{ alertCount }}
        </span>
      </CardHeader>
      <CardContent class="space-y-1 text-sm">
        <div v-if="alertCount === 0" class="flex items-center gap-2 text-muted-foreground">
          <CheckCircle2 class="size-4 text-primary" /> {{ t("dashboard.allClear") }}
        </div>

        <RouterLink v-for="q in nearQuota" :key="`q-${q.id}`" to="/admin/quotas" class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-muted">
          <span><AlertTriangle class="mr-2 inline size-4 text-destructive" />{{ q.name }} — {{ t("dashboard.nearQuota") }}</span>
          <span class="text-muted-foreground">→</span>
        </RouterLink>
        <RouterLink v-for="w in unhealthyWorkers" :key="`w-${w.worker_id}`" to="/admin/workers" class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-muted">
          <span><AlertTriangle class="mr-2 inline size-4 text-destructive" />{{ w.worker_id }} — {{ t("dashboard.unhealthyWorker") }}</span>
          <span class="text-muted-foreground">→</span>
        </RouterLink>
        <RouterLink v-for="e in offlineEdges" :key="`e-${e.edge_id}`" to="/admin/edges" class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-muted">
          <span><AlertTriangle class="mr-2 inline size-4 text-destructive" />{{ e.edge_id }} — {{ t("dashboard.offlineEdge") }}</span>
          <span class="text-muted-foreground">→</span>
        </RouterLink>
        <RouterLink v-for="tn in suspendedTenants" :key="`s-${tn.id}`" to="/admin/tenants" class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-muted">
          <span><AlertTriangle class="mr-2 inline size-4 text-destructive" />{{ tn.name }} — {{ t("dashboard.suspendedTenant") }}</span>
          <span class="text-muted-foreground">→</span>
        </RouterLink>
      </CardContent>
    </Card>
  </div>
</template>
