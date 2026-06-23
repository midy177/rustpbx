<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Did } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const dids = ref<Did[]>([]);
const loading = ref(true);
const error = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
    dids.value = await api.get<Did[]>(`/dids${q}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function statusLabel(s: string) {
  return t(`didsPage.status${s.charAt(0).toUpperCase()}${s.slice(1)}`);
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("didsPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("didsPage.tenantSubtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("didsPage.number") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("didsPage.country") }}</TableHead>
            <TableHead>{{ t("didsPage.city") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="4">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="dids.length === 0" :colspan="4">{{ t("didsPage.noDids") }}</TableEmpty>
          <TableRow v-for="d in dids" :key="d.id">
            <TableCell class="font-mono">{{ d.number }}</TableCell>
            <TableCell><Badge variant="success">{{ statusLabel(d.status) }}</Badge></TableCell>
            <TableCell>{{ d.country ?? "—" }}</TableCell>
            <TableCell>{{ d.city ?? "—" }}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
