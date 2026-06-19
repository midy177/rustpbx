<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Tenant } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

const { t } = useI18n();
const auth = useAuthStore();
const tenant = ref<Tenant | null>(null);

onMounted(async () => {
  if (auth.activeTenantId) {
    tenant.value = await api.get<Tenant>(`/tenants/${auth.activeTenantId}`).catch(() => null);
  }
});
</script>

<template>
  <div class="space-y-6">
    <h2 class="text-2xl font-bold tracking-tight">{{ t("tenantArea.profileTitle") }}</h2>
    <Card v-if="tenant" class="max-w-xl">
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          {{ tenant.name }}
          <Badge variant="success">{{ tenant.status }}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent class="space-y-2 text-sm">
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("common.id") }}</span><span>{{ tenant.id }}</span>
        </div>
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("tenants.maxConcurrentCalls") }}</span>
          <span>{{ tenant.max_concurrent_calls ?? t("common.unlimited") }}</span>
        </div>
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("tenants.storagePrefix") }}</span>
          <span>{{ tenant.storage_prefix ?? "—" }}</span>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
