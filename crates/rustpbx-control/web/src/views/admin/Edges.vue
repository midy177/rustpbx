<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Edge } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const edges = ref<Edge[]>([]);
const loading = ref(true);

function natVariant(n: string) {
  if (n === "open" || n === "cone") return "success" as const;
  if (n === "symmetric" || n === "blocked") return "destructive" as const;
  return "muted" as const;
}

async function load() {
  loading.value = true;
  try {
    edges.value = await api.get<Edge[]>("/edges");
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("edges.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("edges.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("edges.edgeId") }}</TableHead>
            <TableHead>{{ t("edges.sipAddr") }}</TableHead>
            <TableHead>{{ t("edges.transports") }}</TableHead>
            <TableHead>{{ t("edges.region") }}</TableHead>
            <TableHead>{{ t("edges.version") }}</TableHead>
            <TableHead>{{ t("workers.nat") }}</TableHead>
            <TableHead>{{ t("edges.activeCalls") }}</TableHead>
            <TableHead>{{ t("edges.lastHeartbeat") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="9">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="edges.length === 0" :colspan="9">{{ t("edges.noEdges") }}</TableEmpty>
          <TableRow v-for="e in edges" :key="e.edge_id">
            <TableCell class="font-medium">{{ e.edge_id }}</TableCell>
            <TableCell class="font-mono text-xs">{{ e.sip_addr }}</TableCell>
            <TableCell class="uppercase text-xs">{{ e.transports.join(", ") || "—" }}</TableCell>
            <TableCell>{{ e.region || "—" }}</TableCell>
            <TableCell class="font-mono text-xs">{{ e.version || "—" }}</TableCell>
            <TableCell>
              <Badge v-if="e.nat_type" :variant="natVariant(e.nat_type)">{{ e.nat_type }}</Badge>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell>{{ e.active_calls }}</TableCell>
            <TableCell class="text-muted-foreground">
              {{ t("edges.secondsAgo", { n: e.last_heartbeat_secs_ago }) }}
            </TableCell>
            <TableCell>
              <Badge v-if="e.healthy" variant="success">{{ t("edges.healthy") }}</Badge>
              <Badge v-else variant="destructive">{{ t("edges.unhealthy") }}</Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
