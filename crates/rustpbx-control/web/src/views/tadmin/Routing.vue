<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Route, type RouteInput } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Dialog } from "@/components/ui/dialog";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { Plus, Pencil, Trash2, RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();

const routes = ref<Route[]>([]);
const loading = ref(true);
const error = ref("");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";

const form = reactive<{
  name: string;
  description: string;
  direction: string;
  priority: number | string | null;
  is_active: boolean;
  selection_strategy: string;
  hash_key: string;
  source_pattern: string;
  destination_pattern: string;
  target_trunks: string;
}>({
  name: "",
  description: "",
  direction: "any",
  priority: 100,
  is_active: true,
  selection_strategy: "rr",
  hash_key: "",
  source_pattern: "",
  destination_pattern: "",
  target_trunks: "",
});

async function load() {
  loading.value = true;
  error.value = "";
  try {
    routes.value = await api.get<Route[]>(`/routes${q}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function openCreate() {
  editingId.value = null;
  Object.assign(form, {
    name: "",
    description: "",
    direction: "any",
    priority: 100,
    is_active: true,
    selection_strategy: "rr",
    hash_key: "",
    source_pattern: "",
    destination_pattern: "",
    target_trunks: "",
  });
  dialogOpen.value = true;
}

function openEdit(r: Route) {
  editingId.value = r.id;
  Object.assign(form, {
    name: r.name,
    description: r.description ?? "",
    direction: r.direction || "any",
    priority: r.priority,
    is_active: r.is_active,
    selection_strategy: "rr",
    hash_key: "",
    source_pattern: r.source_pattern ?? "",
    destination_pattern: r.destination_pattern ?? "",
    target_trunks: r.target_trunks.join(", "),
  });
  dialogOpen.value = true;
}

function num(v: unknown, fallback: number): number {
  if (v === null || v === undefined || v === "") return fallback;
  const n = Number(v);
  return Number.isNaN(n) ? fallback : n;
}

async function save() {
  if (!form.name.trim()) {
    error.value = t("common.name");
    return;
  }
  saving.value = true;
  error.value = "";
  const payload: RouteInput = {
    name: form.name.trim(),
    description: form.description.trim() || null,
    direction: form.direction,
    priority: num(form.priority, 100),
    is_active: form.is_active,
    selection_strategy: form.selection_strategy,
    hash_key: form.hash_key.trim() || null,
    source_pattern: form.source_pattern.trim() || null,
    destination_pattern: form.destination_pattern.trim() || null,
    target_trunks: form.target_trunks
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0),
  };
  try {
    if (editingId.value) await api.post(`/routes/${editingId.value}`, payload);
    else await api.post(`/routes${q}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(r: Route) {
  if (!confirm(t("routingPage.deleteConfirm", { name: r.name }))) return;
  try {
    await api.del(`/routes/${r.id}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("nav.routing") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("routingPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="auth.can('routing:write')" @click="openCreate">
          <Plus class="size-4" />
          {{ t("routingPage.newRoute") }}
        </Button>
      </div>
    </div>

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
            <TableHead v-if="auth.can('routing:write')" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="8">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="routes.length === 0" :colspan="8">{{ t("routingPage.noRoutes") }}</TableEmpty>
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
              <span v-if="r.target_trunks.length">{{ r.target_trunks.join(", ") }}</span>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell>
              <Badge :variant="r.is_active ? 'success' : 'muted'">
                {{ r.is_active ? t("routingPage.active") : t("common.status") }}
              </Badge>
            </TableCell>
            <TableCell v-if="auth.can('routing:write')" class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(r)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(r)" :aria-label="t('common.delete')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Dialog
      v-model:open="dialogOpen"
      :title="editingId ? t('routingPage.editRoute') : t('routingPage.newRoute')"
    >
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid gap-2">
          <Label for="r-name">{{ t("common.name") }}</Label>
          <Input id="r-name" v-model="form.name" />
        </div>
        <div class="grid gap-2">
          <Label for="r-desc">{{ t("routingPage.description") }} <span class="text-muted-foreground">({{ t("common.optional") }})</span></Label>
          <Input id="r-desc" v-model="form.description" />
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="r-direction">{{ t("routingPage.direction") }}</Label>
            <select
              id="r-direction"
              v-model="form.direction"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="any">any</option>
              <option value="inbound">inbound</option>
              <option value="outbound">outbound</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="r-priority">{{ t("routingPage.priority") }}</Label>
            <Input id="r-priority" v-model="form.priority" type="number" />
          </div>
          <div class="grid gap-2">
            <Label for="r-strategy">{{ t("routingPage.strategy") }}</Label>
            <select
              id="r-strategy"
              v-model="form.selection_strategy"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="rr">round-robin</option>
              <option value="hash">hash</option>
              <option value="weight">weight</option>
              <option value="failover">failover</option>
            </select>
          </div>
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="r-source">{{ t("routingPage.source") }} <span class="text-muted-foreground">({{ t("common.optional") }})</span></Label>
            <Input id="r-source" v-model="form.source_pattern" />
          </div>
          <div class="grid gap-2">
            <Label for="r-destination">{{ t("routingPage.destination") }} <span class="text-muted-foreground">({{ t("common.optional") }})</span></Label>
            <Input id="r-destination" v-model="form.destination_pattern" />
          </div>
        </div>
        <div v-if="form.selection_strategy === 'hash'" class="grid gap-2">
          <Label for="r-hashkey">hash_key <span class="text-muted-foreground">({{ t("common.optional") }})</span></Label>
          <Input id="r-hashkey" v-model="form.hash_key" />
        </div>
        <div class="grid gap-2">
          <Label for="r-targets">{{ t("routingPage.targets") }}</Label>
          <Input id="r-targets" v-model="form.target_trunks" />
          <p class="text-xs text-muted-foreground">{{ t("routingPage.targetsHint") }}</p>
        </div>
        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" v-model="form.is_active" class="size-4 rounded border-input" />
          {{ t("routingPage.isActive") }}
        </label>
      </form>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
