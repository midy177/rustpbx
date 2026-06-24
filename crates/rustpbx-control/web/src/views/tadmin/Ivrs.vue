<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Ivr, type IvrInput } from "@/api/client";
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

const ivrs = ref<Ivr[]>([]);
const loading = ref(true);
const error = ref("");
const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const formName = ref("");
const formDesc = ref("");
const formActive = ref(true);
// Raw JSON editor for the IvrDefinition spec (visual editor is a commercial addon).
const specText = ref("");

const DEFAULT_SPEC = `{
  "name": "main",
  "ivr_mode": "tree",
  "root": {
    "greeting": "sounds/welcome.wav",
    "timeout_ms": 5000,
    "max_retries": 3,
    "entries": [
      { "key": "1", "action": { "type": "transfer", "target": "1001" } },
      { "key": "2", "action": { "type": "queue", "target": "support" } },
      { "key": "0", "action": { "type": "play", "prompt": "sounds/info.wav" } }
    ]
  },
  "menus": {}
}`;

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    ivrs.value = await api.get<Ivr[]>(`/ivrs${scope()}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function openCreate() {
  editingId.value = null;
  formName.value = "";
  formDesc.value = "";
  formActive.value = true;
  specText.value = DEFAULT_SPEC;
  error.value = "";
  dialogOpen.value = true;
}

function openEdit(iv: Ivr) {
  editingId.value = iv.id;
  formName.value = iv.name;
  formDesc.value = iv.description ?? "";
  formActive.value = iv.is_active;
  specText.value = JSON.stringify(iv.spec, null, 2);
  error.value = "";
  dialogOpen.value = true;
}

async function save() {
  if (!formName.value.trim()) {
    error.value = t("ivrsPage.nameRequired");
    return;
  }
  let spec: Record<string, unknown>;
  try {
    spec = JSON.parse(specText.value);
  } catch (e) {
    error.value = t("ivrsPage.invalidJson") + ": " + (e instanceof Error ? e.message : "");
    return;
  }
  const payload: IvrInput = {
    name: formName.value.trim(),
    description: formDesc.value.trim() || null,
    is_active: formActive.value,
    spec,
  };
  saving.value = true;
  error.value = "";
  try {
    if (editingId.value) await api.post(`/ivrs/${editingId.value}`, payload);
    else await api.post(`/ivrs${scope()}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(iv: Ivr) {
  if (!confirm(t("ivrsPage.deleteConfirm", { name: iv.name }))) return;
  try {
    await api.del(`/ivrs/${iv.id}`);
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("ivrsPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("ivrsPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="auth.can('ivr:write')" @click="openCreate">
          <Plus class="size-4" /> {{ t("ivrsPage.newIvr") }}
        </Button>
      </div>
    </div>

    <p v-if="error && !dialogOpen" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("common.name") }}</TableHead>
            <TableHead>{{ t("ivrsPage.mode") }}</TableHead>
            <TableHead>{{ t("ivrsPage.entries") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead v-if="auth.can('ivr:write')" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="5">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="ivrs.length === 0" :colspan="5">{{ t("ivrsPage.noIvrs") }}</TableEmpty>
          <TableRow v-for="iv in ivrs" :key="iv.id">
            <TableCell>
              <div class="font-medium">{{ iv.name }}</div>
              <div v-if="iv.description" class="text-muted-foreground text-xs">{{ iv.description }}</div>
            </TableCell>
            <TableCell>
              <Badge variant="muted">{{ (iv.spec as Record<string, unknown>).ivr_mode ?? "tree" }}</Badge>
            </TableCell>
            <TableCell>{{ ((iv.spec as Record<string, unknown>).root as { entries?: unknown[] })?.entries?.length ?? 0 }}</TableCell>
            <TableCell>
              <Badge :variant="iv.is_active ? 'success' : 'muted'">
                {{ iv.is_active ? t("common.active") : t("common.disabled") }}
              </Badge>
            </TableCell>
            <TableCell v-if="auth.can('ivr:write')" class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(iv)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(iv)" :aria-label="t('common.delete')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Dialog v-model:open="dialogOpen" :title="editingId ? t('ivrsPage.editIvr') : t('ivrsPage.newIvr')">
      <div class="grid gap-4">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="ivr-name">{{ t("common.name") }}</Label>
            <Input id="ivr-name" v-model="formName" placeholder="main" />
            <p class="text-xs text-muted-foreground">{{ t("ivrsPage.nameHint") }}</p>
          </div>
          <div class="grid gap-2">
            <Label for="ivr-desc">{{ t("ivrsPage.description") }}</Label>
            <Input id="ivr-desc" v-model="formDesc" />
          </div>
        </div>

        <div class="grid gap-2">
          <Label for="ivr-spec">{{ t("ivrsPage.spec") }}</Label>
          <textarea
            id="ivr-spec"
            v-model="specText"
            rows="16"
            spellcheck="false"
            class="rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          ></textarea>
          <p class="text-xs text-muted-foreground">{{ t("ivrsPage.specHint") }}</p>
        </div>

        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" class="size-4" v-model="formActive" />
          {{ t("common.active") }}
        </label>

        <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
          <Button :disabled="saving" @click="save">{{ t("common.save") }}</Button>
        </div>
      </div>
    </Dialog>
  </div>
</template>
