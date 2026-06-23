<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Did } from "@/api/client";
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
const dids = ref<Did[]>([]);
const loading = ref(true);
const error = ref("");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const form = reactive({
  number: "",
  tenant_id: null as number | null,
  status: "available",
  country: "",
  city: "",
  monthly_cost: null as number | null,
  unassign: false,
});

function num(v: unknown): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = Number(v);
  return Number.isNaN(n) ? null : n;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    dids.value = await api.get<Did[]>("/dids");
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
    number: "", tenant_id: null, status: "available", country: "", city: "",
    monthly_cost: null, unassign: false,
  });
  dialogOpen.value = true;
}

function openEdit(d: Did) {
  editingId.value = d.id;
  Object.assign(form, {
    number: d.number,
    tenant_id: d.tenant_id,
    status: d.status,
    country: d.country ?? "",
    city: d.city ?? "",
    monthly_cost: d.monthly_cost,
    unassign: false,
  });
  dialogOpen.value = true;
}

async function save() {
  if (!form.number.trim()) {
    error.value = t("didsPage.numberRequired");
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    if (editingId.value) {
      await api.post(`/dids/${editingId.value}`, {
        tenant_id: form.unassign ? null : num(form.tenant_id),
        status: form.status,
        country: form.country || null,
        city: form.city || null,
        monthly_cost: num(form.monthly_cost),
        unassign: form.unassign,
      });
    } else {
      await api.post("/dids", {
        number: form.number.trim(),
        tenant_id: num(form.tenant_id),
        status: form.status,
        country: form.country || null,
        city: form.city || null,
        monthly_cost: num(form.monthly_cost),
      });
    }
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(d: Did) {
  if (!confirm(t("didsPage.deleteConfirm", { number: d.number }))) return;
  try {
    await api.del(`/dids/${d.id}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function statusVariant(s: string) {
  if (s === "assigned") return "success" as const;
  if (s === "available") return "secondary" as const;
  return "muted" as const;
}
function statusLabel(s: string) {
  return t(`didsPage.status${s.charAt(0).toUpperCase()}${s.slice(1)}`);
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("didsPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("didsPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button @click="openCreate">
          <Plus class="size-4" /> {{ t("didsPage.newDid") }}
        </Button>
      </div>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("didsPage.number") }}</TableHead>
            <TableHead>{{ t("didsPage.tenant") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("didsPage.country") }}</TableHead>
            <TableHead>{{ t("didsPage.monthlyCost") }}</TableHead>
            <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="6">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="dids.length === 0" :colspan="6">{{ t("didsPage.noDids") }}</TableEmpty>
          <TableRow v-for="d in dids" :key="d.id">
            <TableCell class="font-mono">{{ d.number }}</TableCell>
            <TableCell>
              <span v-if="d.tenant_id">#{{ d.tenant_id }}</span>
              <span v-else class="text-muted-foreground">{{ t("didsPage.unassigned") }}</span>
            </TableCell>
            <TableCell><Badge :variant="statusVariant(d.status)">{{ statusLabel(d.status) }}</Badge></TableCell>
            <TableCell>{{ d.country ?? "—" }}</TableCell>
            <TableCell>{{ d.monthly_cost ?? "—" }}</TableCell>
            <TableCell class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(d)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(d)" :aria-label="t('common.delete')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Dialog v-model:open="dialogOpen" :title="editingId ? t('didsPage.editDid') : t('didsPage.newDid')">
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid gap-2">
          <Label for="d-num">{{ t("didsPage.number") }}</Label>
          <Input id="d-num" v-model="form.number" :disabled="!!editingId" placeholder="+12025550100" />
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="d-tenant">{{ t("didsPage.assignTenant") }}</Label>
            <Input id="d-tenant" v-model="form.tenant_id" type="number" :disabled="form.unassign" />
          </div>
          <div class="grid gap-2">
            <Label for="d-status">{{ t("common.status") }}</Label>
            <select
              id="d-status"
              v-model="form.status"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="available">{{ t("didsPage.statusAvailable") }}</option>
              <option value="assigned">{{ t("didsPage.statusAssigned") }}</option>
              <option value="reserved">{{ t("didsPage.statusReserved") }}</option>
              <option value="porting">{{ t("didsPage.statusPorting") }}</option>
            </select>
          </div>
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="d-country">{{ t("didsPage.country") }}</Label>
            <Input id="d-country" v-model="form.country" />
          </div>
          <div class="grid gap-2">
            <Label for="d-city">{{ t("didsPage.city") }}</Label>
            <Input id="d-city" v-model="form.city" />
          </div>
          <div class="grid gap-2">
            <Label for="d-cost">{{ t("didsPage.monthlyCost") }}</Label>
            <Input id="d-cost" v-model="form.monthly_cost" type="number" />
          </div>
        </div>
        <label v-if="editingId" class="flex items-center gap-2 text-sm">
          <input type="checkbox" v-model="form.unassign" class="size-4 rounded border-input" />
          {{ t("didsPage.unassign") }}
        </label>
      </form>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
