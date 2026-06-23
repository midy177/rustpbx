<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type CallRecord } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw, PlayCircle } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const records = ref<CallRecord[]>([]);
const loading = ref(true);
const error = ref("");

function fmtDuration(secs: number) {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function statusVariant(s: string) {
  const v = s.toLowerCase();
  if (v.includes("complet") || v.includes("answer")) return "success" as const;
  if (v.includes("fail") || v.includes("reject") || v.includes("error")) return "warning" as const;
  return "muted" as const;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
    records.value = await api.get<CallRecord[]>(`/call-records${q}`);
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("cdrPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("cdrPage.subtitle") }}</p>
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
            <TableHead>{{ t("cdrPage.from") }}</TableHead>
            <TableHead>{{ t("cdrPage.to") }}</TableHead>
            <TableHead>{{ t("cdrPage.direction") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("cdrPage.duration") }}</TableHead>
            <TableHead>{{ t("cdrPage.started") }}</TableHead>
            <TableHead>{{ t("cdrPage.recording") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="7">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="records.length === 0" :colspan="7">{{ t("cdrPage.noRecords") }}</TableEmpty>
          <TableRow v-for="r in records" :key="r.id">
            <TableCell class="font-mono text-xs">{{ r.from_number ?? "—" }}</TableCell>
            <TableCell class="font-mono text-xs">{{ r.to_number ?? "—" }}</TableCell>
            <TableCell>{{ r.direction }}</TableCell>
            <TableCell><Badge :variant="statusVariant(r.status)">{{ r.status }}</Badge></TableCell>
            <TableCell>{{ fmtDuration(r.duration_secs) }}</TableCell>
            <TableCell class="text-muted-foreground text-xs">
              {{ r.started_at ? formatDate(r.started_at) : "—" }}
            </TableCell>
            <TableCell>
              <a
                v-if="r.recording_url"
                :href="r.recording_url"
                target="_blank"
                rel="noopener"
                class="inline-flex items-center gap-1 text-sm text-primary hover:underline"
              >
                <PlayCircle class="size-4" /> {{ t("cdrPage.play") }}
              </a>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
