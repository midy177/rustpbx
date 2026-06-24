<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type RaftMetrics } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw, Plus, Check } from "lucide-vue-next";

const { t } = useI18n();
const metrics = ref<RaftMetrics | null>(null);
const loading = ref(true);
const error = ref("");
const msg = ref("");

// add-learner form
const newNodeId = ref<number | null>(null);
const newAddr = ref("");
const newGrpcAddr = ref("");

// change-membership: which current members are voters
const voters = ref<Set<number>>(new Set());

const stateVariant = (s: string) => {
  if (s === "Leader") return "success" as const;
  if (s === "Candidate") return "warning" as const;
  return "muted" as const;
};

async function load() {
  loading.value = true;
  error.value = "";
  try {
    metrics.value = await api.get<RaftMetrics>("/raft/metrics");
    // Default the voter set to all current members.
    voters.value = new Set(metrics.value.members.map(([id]) => id));
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}

async function addLearner() {
  if (newNodeId.value == null || !newAddr.value.trim()) return;
  msg.value = "";
  try {
    await api.post("/raft/add-learner", {
      node_id: newNodeId.value,
      addr: newAddr.value.trim(),
      grpc_addr: newGrpcAddr.value.trim(),
    });
    msg.value = t("raft.added", { id: newNodeId.value });
    newNodeId.value = null;
    newAddr.value = "";
    newGrpcAddr.value = "";
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function toggleVoter(id: number) {
  const next = new Set(voters.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  voters.value = next;
}

async function applyMembership() {
  msg.value = "";
  try {
    await api.post("/raft/change-membership", { voters: [...voters.value].sort((a, b) => a - b) });
    msg.value = t("raft.membershipApplied");
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

const leaderLabel = computed(() => {
  const l = metrics.value?.current_leader;
  return l == null ? "—" : `#${l}`;
});

onMounted(load);
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("raft.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("raft.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
    <p v-if="msg" class="text-sm text-primary">{{ msg }}</p>

    <!-- Cluster metrics -->
    <div class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-6">
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.node") }}</div>
        <div class="mt-1 font-mono text-lg font-semibold">#{{ metrics?.id ?? "—" }}</div>
      </Card>
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.state") }}</div>
        <div class="mt-1">
          <Badge v-if="metrics" :variant="stateVariant(metrics.state)">{{ metrics.state }}</Badge>
          <span v-else>—</span>
        </div>
      </Card>
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.term") }}</div>
        <div class="mt-1 font-mono text-lg font-semibold">{{ metrics?.current_term ?? "—" }}</div>
      </Card>
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.leader") }}</div>
        <div class="mt-1 font-mono text-lg font-semibold">{{ leaderLabel }}</div>
      </Card>
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.lastLog") }}</div>
        <div class="mt-1 font-mono text-lg font-semibold">{{ metrics?.last_log_index ?? "—" }}</div>
      </Card>
      <Card class="p-4">
        <div class="text-muted-foreground text-xs">{{ t("raft.lastApplied") }}</div>
        <div class="mt-1 font-mono text-lg font-semibold">{{ metrics?.last_applied ?? "—" }}</div>
      </Card>
    </div>

    <!-- Members -->
    <Card>
      <div class="flex items-center justify-between p-4">
        <h3 class="font-semibold">{{ t("raft.members") }}</h3>
        <Button size="sm" :disabled="voters.size === 0" @click="applyMembership">
          <Check class="size-4" /> {{ t("raft.applyMembership") }}
        </Button>
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead class="w-10"></TableHead>
            <TableHead>{{ t("raft.nodeId") }}</TableHead>
            <TableHead>{{ t("raft.addr") }}</TableHead>
            <TableHead>{{ t("raft.role") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="4">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="!metrics || metrics.members.length === 0" :colspan="4">
            {{ t("raft.noMembers") }}
          </TableEmpty>
          <TableRow v-for="[id, addr] in metrics?.members ?? []" :key="id">
            <TableCell>
              <input
                type="checkbox"
                class="size-4"
                :checked="voters.has(id)"
                @change="toggleVoter(id)"
              />
            </TableCell>
            <TableCell class="font-mono">#{{ id }}</TableCell>
            <TableCell class="font-mono text-xs">{{ addr }}</TableCell>
            <TableCell>
              <Badge v-if="metrics?.current_leader === id" variant="success">{{ t("raft.leaderRole") }}</Badge>
              <Badge v-else-if="voters.has(id)" variant="muted">{{ t("raft.voter") }}</Badge>
              <Badge v-else variant="warning">{{ t("raft.learner") }}</Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <!-- Add learner -->
    <Card class="p-4">
      <h3 class="mb-1 font-semibold">{{ t("raft.addLearner") }}</h3>
      <p class="mb-3 text-muted-foreground text-xs">{{ t("raft.addLearnerDesc") }}</p>
      <form class="flex flex-wrap items-end gap-3" @submit.prevent="addLearner">
        <div>
          <label class="mb-1 block text-xs text-muted-foreground">{{ t("raft.learnerNodeId") }}</label>
          <Input v-model.number="newNodeId" type="number" min="1" class="w-28" placeholder="2" />
        </div>
        <div class="min-w-[220px] flex-1">
          <label class="mb-1 block text-xs text-muted-foreground">{{ t("raft.learnerAddr") }}</label>
          <Input v-model="newAddr" placeholder="peer-2.rustpbx-control:9091" />
        </div>
        <div class="min-w-[220px] flex-1">
          <label class="mb-1 block text-xs text-muted-foreground">{{ t("raft.learnerGrpcAddr") }}</label>
          <Input v-model="newGrpcAddr" :placeholder="t('raft.learnerGrpcPlaceholder')" />
        </div>
        <Button type="submit" :disabled="newNodeId == null || !newAddr.trim()">
          <Plus class="size-4" /> {{ t("raft.add") }}
        </Button>
      </form>
    </Card>
  </div>
</template>
