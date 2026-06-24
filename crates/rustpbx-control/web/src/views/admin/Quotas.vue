<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type TenantQuota, type QuotaUsage } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const rows = ref<TenantQuota[]>([]);
const loading = ref(true);
const error = ref("");

/// Saturation % for a capped resource (0 if unlimited).
function pct(u: QuotaUsage): number {
  if (!u.max || u.max === 0) return 0;
  return Math.min(100, Math.round((u.used / u.max) * 100));
}

const NEAR = 80; // % at which a resource is flagged near-limit

async function load() {
  loading.value = true;
  error.value = "";
  try {
    rows.value = await api.get<TenantQuota[]>("/tenant-quotas");
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("quotas.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("quotas.subtitle") }}</p>
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
            <TableHead>{{ t("quotas.tenant") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead class="w-[180px]">{{ t("quotas.trunks") }}</TableHead>
            <TableHead class="w-[180px]">{{ t("quotas.dids") }}</TableHead>
            <TableHead class="w-[180px]">{{ t("quotas.concurrent") }}</TableHead>
            <TableHead class="w-[120px]">{{ t("quotas.saturation") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="6">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="rows.length === 0" :colspan="6">{{ t("quotas.noData") }}</TableEmpty>
          <TableRow v-for="q in rows" :key="q.id">
            <TableCell class="font-medium">{{ q.name }}</TableCell>
            <TableCell>
              <Badge :variant="q.status === 'active' ? 'success' : 'warning'">{{ q.status }}</Badge>
            </TableCell>
            <TableCell>
              <div class="flex items-center gap-2">
                <div class="h-2 w-24 rounded bg-muted overflow-hidden">
                  <div
                    v-if="q.trunks.max"
                    class="h-full"
                    :class="pct(q.trunks) >= NEAR ? 'bg-destructive' : 'bg-primary'"
                    :style="{ width: pct(q.trunks) + '%' }"
                  />
                </div>
                <span class="text-xs tabular-nums">{{ q.trunks.used }}/{{ q.trunks.max ?? "∞" }}</span>
              </div>
            </TableCell>
            <TableCell>
              <div class="flex items-center gap-2">
                <div class="h-2 w-24 rounded bg-muted overflow-hidden">
                  <div
                    v-if="q.dids.max"
                    class="h-full"
                    :class="pct(q.dids) >= NEAR ? 'bg-destructive' : 'bg-primary'"
                    :style="{ width: pct(q.dids) + '%' }"
                  />
                </div>
                <span class="text-xs tabular-nums">{{ q.dids.used }}/{{ q.dids.max ?? "∞" }}</span>
              </div>
            </TableCell>
            <TableCell>
              <div class="flex items-center gap-2">
                <div class="h-2 w-24 rounded bg-muted overflow-hidden">
                  <div
                    v-if="q.concurrent.max"
                    class="h-full"
                    :class="pct(q.concurrent) >= NEAR ? 'bg-destructive' : 'bg-primary'"
                    :style="{ width: pct(q.concurrent) + '%' }"
                  />
                </div>
                <span class="text-xs tabular-nums">{{ q.concurrent.used }}/{{ q.concurrent.max ?? "∞" }}</span>
              </div>
            </TableCell>
            <TableCell>
              <Badge
                v-if="Math.max(pct(q.trunks), pct(q.dids), pct(q.concurrent)) >= NEAR"
                variant="destructive"
              >
                {{ t("quotas.nearLimit") }}
              </Badge>
              <Badge v-else variant="muted">{{ t("quotas.healthy") }}</Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
