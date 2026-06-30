<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Worker, type WorkerContactConflict } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { AlertTriangle, RefreshCw, Pause, Trash2 } from "lucide-vue-next";

const { t, te } = useI18n();
const workers = ref<Worker[]>([]);
const contactConflicts = ref<WorkerContactConflict[]>([]);
const loading = ref(true);
const error = ref("");

/** Localized NAT-type label, falling back to the raw value if unmapped. */
function natLabel(n: string) {
  const key = `workers.natTypes.${n}`;
  return te(key) ? t(key) : n;
}

async function drainWorker(w: Worker) {
  if (!confirm(t("workers.drainConfirm", { id: w.worker_id }))) return;
  error.value = "";
  try {
    await api.post(`/workers/${encodeURIComponent(w.worker_id)}/drain`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

async function removeWorker(w: Worker) {
  if (!confirm(t("workers.removeConfirm", { id: w.worker_id }))) return;
  error.value = "";
  try {
    await api.del(`/workers/${encodeURIComponent(w.worker_id)}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function natVariant(n: string) {
  // symmetric / firewall / blocked need a media relay (TURN) → flag red.
  if (n === "symmetric" || n === "firewall" || n === "blocked") return "destructive" as const;
  // open + any *cone (full/restricted/port-restricted) support P2P media.
  if (n === "open" || n.endsWith("cone")) return "success" as const;
  return "muted" as const; // nat / unknown
}

async function load() {
  loading.value = true;
  try {
    const [workerRows, conflictRows] = await Promise.all([
      api.get<Worker[]>("/workers"),
      api.get<WorkerContactConflict[]>("/workers/contact-conflicts"),
    ]);
    workers.value = workerRows;
    contactConflicts.value = conflictRows;
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

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card v-if="contactConflicts.length > 0" class="border-amber-500/30">
      <div class="border-b px-4 py-3">
        <h3 class="flex items-center gap-2 text-sm font-semibold">
          <AlertTriangle class="size-4 text-amber-500" />
          {{ t("workers.contactConflicts") }}
        </h3>
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("workers.affinityKey") }}</TableHead>
            <TableHead>{{ t("workers.contact") }}</TableHead>
            <TableHead>{{ t("workers.selectedWorker") }}</TableHead>
            <TableHead>{{ t("workers.candidates") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="c in contactConflicts" :key="`${c.affinity_key}:${c.contact_key}`">
            <TableCell class="font-mono text-xs">{{ c.affinity_key }}</TableCell>
            <TableCell class="font-mono text-xs">{{ c.contact_key }}</TableCell>
            <TableCell>
              <Badge variant="warning">{{ c.selected_worker_id }}</Badge>
            </TableCell>
            <TableCell class="space-y-1">
              <div v-for="candidate in c.candidates" :key="`${c.affinity_key}:${candidate.worker_id}:${candidate.contact}`" class="font-mono text-xs">
                {{ candidate.worker_id }} · q={{ (candidate.q_milli / 1000).toFixed(3) }} · {{ candidate.contact }}
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("workers.workerId") }}</TableHead>
            <TableHead>{{ t("workers.sipAddr") }}</TableHead>
            <TableHead>{{ t("workers.rtpIp") }}</TableHead>
            <TableHead>{{ t("workers.nat") }}</TableHead>
            <TableHead>{{ t("workers.failureDomain") }}</TableHead>
            <TableHead>{{ t("workers.cost") }}</TableHead>
            <TableHead>{{ t("workers.load") }}</TableHead>
            <TableHead>{{ t("workers.cpu") }}</TableHead>
            <TableHead>{{ t("workers.lastHeartbeat") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="11">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="workers.length === 0" :colspan="11">{{ t("workers.noWorkers") }}</TableEmpty>
          <TableRow v-for="w in workers" :key="w.worker_id">
            <TableCell class="font-medium">{{ w.worker_id }}</TableCell>
            <TableCell class="font-mono text-xs">{{ w.sip_addr }}</TableCell>
            <TableCell class="font-mono text-xs">{{ w.rtp_external_ip }}</TableCell>
            <TableCell>
              <Badge v-if="w.nat_type" :variant="natVariant(w.nat_type)" :title="w.nat_type">{{ natLabel(w.nat_type) }}</Badge>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell class="font-mono text-xs">{{ w.failure_domain }}</TableCell>
            <TableCell>{{ w.schedule_cost }}</TableCell>
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
            <TableCell class="text-right">
              <div class="flex justify-end gap-1">
                <Button
                  v-if="!w.draining"
                  variant="ghost"
                  size="sm"
                  @click="drainWorker(w)"
                  :aria-label="t('workers.drain')"
                >
                  <Pause class="size-4" /> {{ t("workers.drain") }}
                </Button>
                <Button variant="ghost" size="icon" @click="removeWorker(w)" :aria-label="t('workers.remove')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
