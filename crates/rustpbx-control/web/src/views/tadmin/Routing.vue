<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Route } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw, Lock } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const routes = ref<Route[]>([]);
const loading = ref(true);
const error = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
    routes.value = await api.get<Route[]>(`/routes${q}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("nav.routing") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("routingPage.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <p class="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Lock class="size-3" /> {{ t("routingPage.readonly") }}
    </p>
    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead class="w-20">{{ t("routingPage.priority") }}</TableHead>
            <TableHead>{{ t("common.name") }}</TableHead>
            <TableHead>{{ t("routingPage.direction") }}</TableHead>
            <TableHead>{{ t("routingPage.source") }}</TableHead>
            <TableHead>{{ t("routingPage.destination") }}</TableHead>
            <TableHead>{{ t("routingPage.targets") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="7">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="routes.length === 0" :colspan="7">{{ t("routingPage.noRoutes") }}</TableEmpty>
          <TableRow v-for="r in routes" :key="r.id">
            <TableCell class="font-mono">{{ r.priority }}</TableCell>
            <TableCell class="font-medium">
              {{ r.name }}
              <div v-if="r.description" class="text-xs text-muted-foreground">{{ r.description }}</div>
            </TableCell>
            <TableCell>{{ r.direction }}</TableCell>
            <TableCell class="font-mono text-xs">{{ r.source_pattern ?? "*" }}</TableCell>
            <TableCell class="font-mono text-xs">{{ r.destination_pattern ?? "*" }}</TableCell>
            <TableCell>
              <span v-if="r.target_trunks.length" class="flex flex-wrap gap-1">
                <Badge v-for="tn in r.target_trunks" :key="tn" variant="secondary">{{ tn }}</Badge>
              </span>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell>
              <Badge :variant="r.is_active ? 'success' : 'muted'">
                {{ r.is_active ? t("routingPage.active") : t("trunksPage.inactive") }}
              </Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
