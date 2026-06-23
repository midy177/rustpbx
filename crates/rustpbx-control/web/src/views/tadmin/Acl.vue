<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type AclRule } from "@/api/client";
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
const rules = ref<AclRule[]>([]);
const loading = ref(true);
const error = ref("");
const canWrite = auth.can("acl:write");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const form = reactive({
  action: "allow",
  target: "",
  priority: 100 as number | null,
  is_active: true,
});

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}
function num(v: unknown): number {
  const n = Number(v);
  return Number.isNaN(n) ? 100 : n;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    rules.value = await api.get<AclRule[]>(`/acl${scope()}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function openCreate() {
  error.value = "";
  editingId.value = null;
  Object.assign(form, { action: "allow", target: "", priority: 100, is_active: true });
  dialogOpen.value = true;
}
function openEdit(r: AclRule) {
  error.value = "";
  editingId.value = r.id;
  Object.assign(form, {
    action: r.action,
    target: r.target,
    priority: r.priority,
    is_active: r.is_active,
  });
  dialogOpen.value = true;
}

async function save() {
  if (!form.target.trim()) {
    error.value = t("aclPage.targetRequired");
    return;
  }
  saving.value = true;
  error.value = "";
  const payload = {
    action: form.action,
    target: form.target.trim(),
    priority: num(form.priority),
    is_active: form.is_active,
  };
  try {
    if (editingId.value) await api.post(`/acl/${editingId.value}`, payload);
    else await api.post(`/acl${scope()}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(r: AclRule) {
  if (!confirm(t("aclPage.deleteConfirm", { action: r.action, target: r.target }))) return;
  try {
    await api.del(`/acl/${r.id}`);
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("aclPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("aclPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="canWrite" @click="openCreate">
          <Plus class="size-4" /> {{ t("aclPage.newRule") }}
        </Button>
      </div>
    </div>

    <p v-if="error && !dialogOpen" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("aclPage.priority") }}</TableHead>
            <TableHead>{{ t("aclPage.action") }}</TableHead>
            <TableHead>{{ t("aclPage.target") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead v-if="canWrite" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="canWrite ? 5 : 4">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="rules.length === 0" :colspan="canWrite ? 5 : 4">{{ t("aclPage.noRules") }}</TableEmpty>
          <TableRow v-for="r in rules" :key="r.id">
            <TableCell class="text-muted-foreground">{{ r.priority }}</TableCell>
            <TableCell>
              <Badge :variant="r.action === 'allow' ? 'success' : 'destructive'">
                {{ r.action === "allow" ? t("aclPage.allow") : t("aclPage.deny") }}
              </Badge>
            </TableCell>
            <TableCell class="font-mono text-xs">
              {{ r.target }}
              <Badge v-if="r.tenant_id === null" variant="muted" class="ml-1">{{ t("aclPage.shared") }}</Badge>
            </TableCell>
            <TableCell>
              <Badge :variant="r.is_active ? 'success' : 'muted'">
                {{ r.is_active ? t("aclPage.active") : t("common.disabled") }}
              </Badge>
            </TableCell>
            <TableCell v-if="canWrite" class="text-right">
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

    <Dialog v-model:open="dialogOpen" :title="editingId ? t('aclPage.editRule') : t('aclPage.newRule')">
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="a-action">{{ t("aclPage.action") }}</Label>
            <select
              id="a-action"
              v-model="form.action"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="allow">{{ t("aclPage.allow") }}</option>
              <option value="deny">{{ t("aclPage.deny") }}</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="a-prio">{{ t("aclPage.priority") }}</Label>
            <Input id="a-prio" v-model="form.priority" type="number" />
          </div>
        </div>
        <div class="grid gap-2">
          <Label for="a-target">{{ t("aclPage.target") }}</Label>
          <Input id="a-target" v-model="form.target" placeholder="10.0.0.0/8" :class="{ 'border-destructive': !form.target.trim() && form.target.length > 0 }" />
          <p class="text-xs text-muted-foreground">{{ t("aclPage.targetHint") }}</p>
        </div>
        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" v-model="form.is_active" class="size-4 rounded border-input" />
          {{ t("aclPage.active") }}
        </label>
      </form>

      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving || !form.target.trim()" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
