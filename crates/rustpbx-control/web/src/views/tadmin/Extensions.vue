<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Extension, type ExtensionInput } from "@/api/client";
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

const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";

const extensions = ref<Extension[]>([]);
const loading = ref(true);
const error = ref("");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const form = reactive<ExtensionInput & { call_forwarding_timeout: number | string | null }>({
  extension: "",
  display_name: "",
  email: "",
  status: "enabled",
  login_disabled: false,
  voicemail_disabled: false,
  allow_guest_calls: false,
  sip_password: "",
  call_forwarding_mode: "none",
  call_forwarding_destination: "",
  call_forwarding_timeout: "",
});

async function load() {
  loading.value = true;
  error.value = "";
  try {
    extensions.value = await api.get<Extension[]>(`/extensions${q}`);
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
  Object.assign(form, {
    extension: "",
    display_name: "",
    email: "",
    status: "enabled",
    login_disabled: false,
    voicemail_disabled: false,
    allow_guest_calls: false,
    sip_password: "",
    call_forwarding_mode: "none",
    call_forwarding_destination: "",
    call_forwarding_timeout: "",
  });
  dialogOpen.value = true;
}

function openEdit(ext: Extension) {
  error.value = "";
  editingId.value = ext.id;
  Object.assign(form, {
    extension: ext.extension,
    display_name: ext.display_name ?? "",
    email: ext.email ?? "",
    status: ext.status ?? "enabled",
    login_disabled: ext.login_disabled,
    voicemail_disabled: ext.voicemail_disabled,
    allow_guest_calls: ext.allow_guest_calls,
    sip_password: "",
    call_forwarding_mode: ext.call_forwarding_mode ?? "none",
    call_forwarding_destination: ext.call_forwarding_destination ?? "",
    call_forwarding_timeout: ext.call_forwarding_timeout ?? "",
  });
  dialogOpen.value = true;
}

function num(v: unknown): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = Number(v);
  return Number.isNaN(n) ? null : n;
}

async function save() {
  if (!form.extension.trim()) {
    error.value = t("extensionsPage.extension");
    return;
  }
  saving.value = true;
  error.value = "";
  const payload: ExtensionInput = {
    extension: form.extension.trim(),
    display_name: form.display_name || null,
    email: form.email || null,
    status: form.status || null,
    login_disabled: form.login_disabled,
    voicemail_disabled: form.voicemail_disabled,
    allow_guest_calls: form.allow_guest_calls,
    call_forwarding_mode: form.call_forwarding_mode || null,
    call_forwarding_destination: form.call_forwarding_destination || null,
    call_forwarding_timeout: num(form.call_forwarding_timeout),
  };
  if (form.sip_password) payload.sip_password = form.sip_password;
  try {
    if (editingId.value) await api.post(`/extensions/${editingId.value}`, payload);
    else await api.post(`/extensions${q}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(ext: Extension) {
  if (!confirm(t("extensionsPage.deleteConfirm", { name: ext.extension }))) return;
  try {
    await api.del(`/extensions/${ext.id}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function statusVariant(s: string | null) {
  if (s === "enabled") return "success" as const;
  if (s === "disabled") return "muted" as const;
  return "muted" as const;
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("extensionsPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("extensionsPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="auth.can('extensions:write')" @click="openCreate">
          <Plus class="size-4" />
          {{ t("extensionsPage.newExtension") }}
        </Button>
      </div>
    </div>

    <p v-if="error && !dialogOpen" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("extensionsPage.extension") }}</TableHead>
            <TableHead>{{ t("extensionsPage.displayName") }}</TableHead>
            <TableHead>{{ t("extensionsPage.email") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("extensionsPage.forwarding") }}</TableHead>
            <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="6">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="extensions.length === 0" :colspan="6">{{ t("extensionsPage.noExtensions") }}</TableEmpty>
          <TableRow v-for="ext in extensions" :key="ext.id">
            <TableCell class="font-medium">{{ ext.extension }}</TableCell>
            <TableCell>{{ ext.display_name ?? "—" }}</TableCell>
            <TableCell class="text-muted-foreground">{{ ext.email ?? "—" }}</TableCell>
            <TableCell><Badge :variant="statusVariant(ext.status)">{{ ext.status ?? "—" }}</Badge></TableCell>
            <TableCell>{{ ext.call_forwarding_mode ?? "—" }}</TableCell>
            <TableCell class="text-right">
              <div v-if="auth.can('extensions:write')" class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(ext)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(ext)" :aria-label="t('common.delete')">
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
      :title="editingId ? t('extensionsPage.editExtension') : t('extensionsPage.newExtension')"
    >
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="e-ext">{{ t("extensionsPage.extension") }}</Label>
            <Input id="e-ext" v-model="form.extension" :class="{ 'border-destructive': !form.extension.trim() && form.extension.length > 0 }" />
          </div>
          <div class="grid gap-2">
            <Label for="e-dn">{{ t("extensionsPage.displayName") }}</Label>
            <Input id="e-dn" v-model="form.display_name" />
          </div>
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="e-email">{{ t("extensionsPage.email") }}</Label>
            <Input id="e-email" v-model="form.email" />
          </div>
          <div class="grid gap-2">
            <Label for="e-status">{{ t("extensionsPage.status") }}</Label>
            <select
              id="e-status"
              v-model="form.status"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="enabled">enabled</option>
              <option value="disabled">disabled</option>
            </select>
          </div>
        </div>
        <div class="grid gap-2">
          <Label for="e-pw">{{ editingId ? t("extensionsPage.sipPasswordKeep") : t("extensionsPage.sipPassword") }}</Label>
          <Input id="e-pw" v-model="form.sip_password" type="password" autocomplete="new-password" />
        </div>
        <div class="flex flex-wrap gap-4">
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.login_disabled" class="size-4 rounded border-input" />
            {{ t("extensionsPage.loginDisabled") }}
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.voicemail_disabled" class="size-4 rounded border-input" />
            {{ t("extensionsPage.voicemailDisabled") }}
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.allow_guest_calls" class="size-4 rounded border-input" />
            {{ t("extensionsPage.allowGuestCalls") }}
          </label>
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="e-cfm">{{ t("extensionsPage.forwardingMode") }}</Label>
            <select
              id="e-cfm"
              v-model="form.call_forwarding_mode"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="none">{{ t("common.none") }}</option>
              <option value="always">always</option>
              <option value="busy">busy</option>
              <option value="no_answer">no_answer</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="e-cfd">{{ t("extensionsPage.forwardingDest") }}</Label>
            <Input id="e-cfd" v-model="form.call_forwarding_destination" />
          </div>
          <div class="grid gap-2">
            <Label for="e-cft">{{ t("extensionsPage.forwardingTimeout") }}</Label>
            <Input id="e-cft" v-model="form.call_forwarding_timeout" type="number" />
          </div>
        </div>
      </form>

      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving || !form.extension.trim()" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
