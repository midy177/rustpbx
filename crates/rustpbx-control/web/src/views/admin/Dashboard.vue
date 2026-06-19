<script setup lang="ts">
import { ref, onMounted, type Component } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Stats } from "@/api/client";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Building2, Server, ServerCog, PhoneCall } from "lucide-vue-next";

const { t } = useI18n();
const stats = ref<Stats | null>(null);
const loading = ref(true);

interface Metric {
  key: string;
  value: () => number | string;
  icon: Component;
}
const metrics: Metric[] = [
  { key: "dashboard.tenants", value: () => stats.value?.tenants ?? 0, icon: Building2 },
  { key: "dashboard.workersHealthy", value: () => stats.value?.workers_healthy ?? 0, icon: Server },
  { key: "dashboard.workersTotal", value: () => stats.value?.workers_total ?? 0, icon: ServerCog },
  { key: "dashboard.activeCalls", value: () => stats.value?.active_calls ?? 0, icon: PhoneCall },
];

onMounted(async () => {
  try {
    stats.value = await api.get<Stats>("/stats");
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
  </div>
</template>
