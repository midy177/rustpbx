<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Worker } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const workers = ref<Worker[]>([]);
const loading = ref(true);

async function load() {
  loading.value = true;
  try {
    workers.value = await api.get<Worker[]>("/workers");
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("workers.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("workers.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("workers.workerId") }}</TableHead>
            <TableHead>{{ t("workers.sipAddr") }}</TableHead>
            <TableHead>{{ t("workers.rtpIp") }}</TableHead>
            <TableHead>{{ t("workers.load") }}</TableHead>
            <TableHead>{{ t("workers.cpu") }}</TableHead>
            <TableHead>{{ t("workers.lastHeartbeat") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="7">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="workers.length === 0" :colspan="7">{{ t("workers.noWorkers") }}</TableEmpty>
          <TableRow v-for="w in workers" :key="w.worker_id">
            <TableCell class="font-medium">{{ w.worker_id }}</TableCell>
            <TableCell class="font-mono text-xs">{{ w.sip_addr }}</TableCell>
            <TableCell class="font-mono text-xs">{{ w.rtp_external_ip }}</TableCell>
            <TableCell>{{ w.active_calls }} / {{ w.max_concurrent }}</TableCell>
            <TableCell>{{ w.cpu_usage.toFixed(1) }}%</TableCell>
            <TableCell class="text-muted-foreground">
              {{ t("workers.secondsAgo", { n: w.last_heartbeat_secs_ago }) }}
            </TableCell>
            <TableCell>
              <Badge v-if="w.draining" variant="warning">{{ t("workers.draining") }}</Badge>
              <Badge v-else-if="w.healthy" variant="success">{{ t("workers.healthy") }}</Badge>
              <Badge v-else variant="destructive">{{ t("workers.unhealthy") }}</Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
