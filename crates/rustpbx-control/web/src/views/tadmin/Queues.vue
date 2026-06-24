<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Queue, type QueueSpec, type QueueInput, type QueueTarget } from "@/api/client";
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

const queues = ref<Queue[]>([]);
const loading = ref(true);
const error = ref("");
const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

interface Form {
  name: string;
  description: string;
  is_active: boolean;
  mode: "sequential" | "parallel";
  wait_timeout: number | null;
  accept_immediately: boolean;
  passthrough_ringback: boolean;
  targets: { uri: string; label: string }[];
}
const form = reactive<Form>(blankForm());

function blankForm(): Form {
  return {
    name: "",
    description: "",
    is_active: true,
    mode: "sequential",
    wait_timeout: 20,
    accept_immediately: true,
    passthrough_ringback: false,
    targets: [{ uri: "", label: "" }],
  };
}

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    queues.value = await api.get<Queue[]>(`/queues${scope()}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function openCreate() {
  editingId.value = null;
  Object.assign(form, blankForm());
  error.value = "";
  dialogOpen.value = true;
}

function openEdit(q: Queue) {
  editingId.value = q.id;
  const s = q.spec;
  Object.assign(form, {
    name: q.name,
    description: q.description ?? "",
    is_active: q.is_active,
    mode: s.strategy?.mode ?? "sequential",
    wait_timeout: s.strategy?.wait_timeout_secs ?? 20,
    accept_immediately: s.accept_immediately ?? true,
    passthrough_ringback: s.passthrough_ringback ?? false,
    targets: (s.strategy?.targets ?? []).map((tg) => ({ uri: tg.uri, label: tg.label ?? "" })),
  });
  if (form.targets.length === 0) form.targets.push({ uri: "", label: "" });
  error.value = "";
  dialogOpen.value = true;
}

function addTarget() {
  form.targets.push({ uri: "", label: "" });
}
function removeTarget(i: number) {
  form.targets.splice(i, 1);
}

async function save() {
  if (!form.name.trim()) {
    error.value = t("queuesPage.nameRequired");
    return;
  }
  const targets: QueueTarget[] = form.targets
    .filter((x) => x.uri.trim())
    .map((x) => ({ uri: x.uri.trim(), label: x.label.trim() || null }));
  if (targets.length === 0) {
    error.value = t("queuesPage.needTarget");
    return;
  }
  const spec: QueueSpec = {
    name: form.name.trim(),
    accept_immediately: form.accept_immediately,
    passthrough_ringback: form.passthrough_ringback,
    strategy: {
      mode: form.mode,
      wait_timeout_secs: form.wait_timeout,
      targets,
    },
  };
  const payload: QueueInput = {
    name: form.name.trim(),
    description: form.description.trim() || null,
    is_active: form.is_active,
    spec,
  };
  saving.value = true;
  error.value = "";
  try {
    if (editingId.value) await api.post(`/queues/${editingId.value}`, payload);
    else await api.post(`/queues${scope()}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(q: Queue) {
  if (!confirm(t("queuesPage.deleteConfirm", { name: q.name }))) return;
  try {
    await api.del(`/queues/${q.id}`);
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("queuesPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("queuesPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="auth.can('queue:write')" @click="openCreate">
          <Plus class="size-4" /> {{ t("queuesPage.newQueue") }}
        </Button>
      </div>
    </div>

    <p v-if="error && !dialogOpen" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("common.name") }}</TableHead>
            <TableHead>{{ t("queuesPage.mode") }}</TableHead>
            <TableHead>{{ t("queuesPage.agents") }}</TableHead>
            <TableHead>{{ t("queuesPage.ringTimeout") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead v-if="auth.can('queue:write')" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="6">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="queues.length === 0" :colspan="6">{{ t("queuesPage.noQueues") }}</TableEmpty>
          <TableRow v-for="q in queues" :key="q.id">
            <TableCell>
              <div class="font-medium">{{ q.name }}</div>
              <div v-if="q.description" class="text-muted-foreground text-xs">{{ q.description }}</div>
            </TableCell>
            <TableCell><Badge variant="muted">{{ q.spec.strategy?.mode ?? "—" }}</Badge></TableCell>
            <TableCell>{{ q.spec.strategy?.targets?.length ?? 0 }}</TableCell>
            <TableCell>{{ q.spec.strategy?.wait_timeout_secs ?? "—" }}s</TableCell>
            <TableCell>
              <Badge :variant="q.is_active ? 'success' : 'muted'">
                {{ q.is_active ? t("common.active") : t("common.disabled") }}
              </Badge>
            </TableCell>
            <TableCell v-if="auth.can('queue:write')" class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(q)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(q)" :aria-label="t('common.delete')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Dialog v-model:open="dialogOpen" :title="editingId ? t('queuesPage.editQueue') : t('queuesPage.newQueue')">
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="q-name">{{ t("common.name") }}</Label>
            <Input id="q-name" v-model="form.name" placeholder="support" />
          </div>
          <div class="grid gap-2">
            <Label for="q-desc">{{ t("queuesPage.description") }}</Label>
            <Input id="q-desc" v-model="form.description" />
          </div>
        </div>

        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="q-mode">{{ t("queuesPage.mode") }}</Label>
            <select id="q-mode" v-model="form.mode" class="h-9 rounded-md border border-input bg-transparent px-3 text-sm">
              <option value="sequential">{{ t("queuesPage.sequential") }}</option>
              <option value="parallel">{{ t("queuesPage.parallel") }}</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="q-timeout">{{ t("queuesPage.ringTimeout") }} (s)</Label>
            <Input id="q-timeout" v-model.number="form.wait_timeout" type="number" min="0" />
          </div>
          <div class="flex items-end gap-4">
            <label class="flex items-center gap-2 text-sm">
              <input type="checkbox" class="size-4" v-model="form.accept_immediately" />
              {{ t("queuesPage.acceptImmediately") }}
            </label>
          </div>
        </div>

        <div class="grid gap-2">
          <div class="flex items-center justify-between">
            <Label>{{ t("queuesPage.agents") }}</Label>
            <Button type="button" variant="outline" size="sm" @click="addTarget">
              <Plus class="size-4" /> {{ t("queuesPage.addAgent") }}
            </Button>
          </div>
          <div v-for="(tg, i) in form.targets" :key="i" class="flex items-center gap-2">
            <Input v-model="tg.uri" placeholder="sip:1001@acme.com" class="flex-1" />
            <Input v-model="tg.label" :placeholder="t('queuesPage.agentLabel')" class="w-40" />
            <Button type="button" variant="ghost" size="icon" @click="removeTarget(i)" :aria-label="t('common.delete')">
              <Trash2 class="size-4 text-destructive" />
            </Button>
          </div>
        </div>

        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" class="size-4" v-model="form.is_active" />
          {{ t("common.active") }}
        </label>

        <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

        <div class="flex justify-end gap-2">
          <Button type="button" variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
          <Button type="submit" :disabled="saving">{{ t("common.save") }}</Button>
        </div>
      </form>
    </Dialog>
  </div>
</template>
