<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { api, type Tenant, type CreateTenant } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Dialog } from "@/components/ui/dialog";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { Plus, Pencil, Trash2, LogIn, RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();

const tenants = ref<Tenant[]>([]);
const loading = ref(true);
const error = ref("");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const form = reactive<CreateTenant & { status?: string }>({
  name: "",
  max_concurrent_calls: null,
  max_trunks: null,
  max_dids: null,
  storage_prefix: null,
});

async function load() {
  loading.value = true;
  error.value = "";
  try {
    tenants.value = await api.get<Tenant[]>("/tenants");
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
    max_concurrent_calls: null,
    max_trunks: null,
    max_dids: null,
    storage_prefix: null,
    status: undefined,
  });
  dialogOpen.value = true;
}

function openEdit(tn: Tenant) {
  editingId.value = tn.id;
  Object.assign(form, {
    name: tn.name,
    max_concurrent_calls: tn.max_concurrent_calls,
    max_trunks: tn.max_trunks,
    max_dids: tn.max_dids,
    storage_prefix: tn.storage_prefix,
    status: tn.status,
  });
  dialogOpen.value = true;
}

function num(v: unknown): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = Number(v);
  return Number.isNaN(n) ? null : n;
}

async function save() {
  if (!form.name.trim()) {
    error.value = t("tenants.nameRequired");
    return;
  }
  saving.value = true;
  error.value = "";
  const payload = {
    name: form.name.trim(),
    max_concurrent_calls: num(form.max_concurrent_calls),
    max_trunks: num(form.max_trunks),
    max_dids: num(form.max_dids),
    storage_prefix: form.storage_prefix || null,
    ...(editingId.value ? { status: form.status } : {}),
  };
  try {
    if (editingId.value) await api.put(`/tenants/${editingId.value}`, payload);
    else await api.post("/tenants", payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(tn: Tenant) {
  if (!confirm(t("tenants.deleteConfirm", { name: tn.name }))) return;
  try {
    await api.del(`/tenants/${tn.id}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function enter(tn: Tenant) {
  auth.setActiveTenant(tn.id);
  router.push("/t/admin/dashboard");
}

function statusVariant(s: string) {
  if (s === "active") return "success" as const;
  if (s === "suspended") return "warning" as const;
  return "muted" as const;
}
function statusLabel(s: string) {
  return t(`tenants.status${s.charAt(0).toUpperCase()}${s.slice(1)}`);
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("tenants.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("tenants.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button @click="openCreate">
          <Plus class="size-4" />
          {{ t("tenants.newTenant") }}
        </Button>
      </div>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead class="w-12">{{ t("common.id") }}</TableHead>
            <TableHead>{{ t("common.name") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("tenants.maxConcurrentCalls") }}</TableHead>
            <TableHead>{{ t("tenants.maxTrunks") }}</TableHead>
            <TableHead>{{ t("tenants.maxDids") }}</TableHead>
            <TableHead>{{ t("common.createdAt") }}</TableHead>
            <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="8">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="tenants.length === 0" :colspan="8">{{ t("common.empty") }}</TableEmpty>
          <TableRow v-for="tn in tenants" :key="tn.id">
            <TableCell class="text-muted-foreground">{{ tn.id }}</TableCell>
            <TableCell class="font-medium">{{ tn.name }}</TableCell>
            <TableCell><Badge :variant="statusVariant(tn.status)">{{ statusLabel(tn.status) }}</Badge></TableCell>
            <TableCell>{{ tn.max_concurrent_calls ?? t("common.unlimited") }}</TableCell>
            <TableCell>{{ tn.max_trunks ?? t("common.unlimited") }}</TableCell>
            <TableCell>{{ tn.max_dids ?? t("common.unlimited") }}</TableCell>
            <TableCell class="text-muted-foreground">{{ formatDate(tn.created_at) }}</TableCell>
            <TableCell class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="sm" @click="enter(tn)">
                  <LogIn class="size-4" /> {{ t("tenants.enter") }}
                </Button>
                <Button variant="ghost" size="icon" @click="openEdit(tn)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(tn)" :aria-label="t('common.delete')">
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
      :title="editingId ? t('tenants.editTenant') : t('tenants.newTenant')"
    >
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid gap-2">
          <Label for="t-name">{{ t("common.name") }}</Label>
          <Input id="t-name" v-model="form.name" :placeholder="t('tenants.namePlaceholder')" />
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="t-mcc">{{ t("tenants.maxConcurrentCalls") }}</Label>
            <Input id="t-mcc" v-model="form.max_concurrent_calls" type="number" />
          </div>
          <div class="grid gap-2">
            <Label for="t-mt">{{ t("tenants.maxTrunks") }}</Label>
            <Input id="t-mt" v-model="form.max_trunks" type="number" />
          </div>
          <div class="grid gap-2">
            <Label for="t-md">{{ t("tenants.maxDids") }}</Label>
            <Input id="t-md" v-model="form.max_dids" type="number" />
          </div>
        </div>
        <div class="grid gap-2">
          <Label for="t-sp">{{ t("tenants.storagePrefix") }}</Label>
          <Input id="t-sp" v-model="form.storage_prefix" />
        </div>
        <div v-if="editingId" class="grid gap-2">
          <Label for="t-status">{{ t("common.status") }}</Label>
          <select
            id="t-status"
            v-model="form.status"
            class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <option value="active">{{ t("tenants.statusActive") }}</option>
            <option value="suspended">{{ t("tenants.statusSuspended") }}</option>
          </select>
        </div>
      </form>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
