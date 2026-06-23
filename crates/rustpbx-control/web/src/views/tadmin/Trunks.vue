<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Trunk, type TrunkInput } from "@/api/client";
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

const trunks = ref<Trunk[]>([]);
const loading = ref(true);
const error = ref("");

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

// Comma-separated text proxies for the array fields.
const didsText = ref("");
const allowedIpsText = ref("");

const form = reactive<TrunkInput>({
  name: "",
  display_name: null,
  carrier: null,
  direction: "bidirectional",
  sip_server: null,
  sip_transport: "udp",
  outbound_proxy: null,
  auth_username: null,
  auth_password: null,
  max_cps: null,
  max_concurrent: null,
  allowed_ips: [],
  did_numbers: [],
  incoming_from_user_prefix: null,
  incoming_to_user_prefix: null,
  is_active: true,
  register_enabled: false,
  register_expires: null,
  rewrite_hostport: true,
});

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const q = scope();
    trunks.value = await api.get<Trunk[]>(`/trunks${q}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

function resetForm() {
  Object.assign(form, {
    name: "",
    display_name: null,
    carrier: null,
    direction: "bidirectional",
    sip_server: null,
    sip_transport: "udp",
    outbound_proxy: null,
    auth_username: null,
    auth_password: null,
    max_cps: null,
    max_concurrent: null,
    allowed_ips: [],
    did_numbers: [],
    incoming_from_user_prefix: null,
    incoming_to_user_prefix: null,
    is_active: true,
    register_enabled: false,
    register_expires: null,
    rewrite_hostport: true,
  } satisfies TrunkInput);
  didsText.value = "";
  allowedIpsText.value = "";
}

function openCreate() {
  error.value = "";
  editingId.value = null;
  resetForm();
  dialogOpen.value = true;
}

function openEdit(tk: Trunk) {
  error.value = "";
  editingId.value = tk.id;
  resetForm();
  // The list Trunk only carries a subset of input fields; prefill what we have
  // and leave the rest blank. Secrets are write-only and never returned.
  Object.assign(form, {
    name: tk.name,
    direction: tk.direction || "bidirectional",
    sip_transport: tk.transport || "udp",
    sip_server: tk.dest,
    max_concurrent: tk.max_concurrent,
    is_active: tk.is_active,
    register_enabled: tk.register_enabled,
  });
  didsText.value = tk.did_numbers.join(", ");
  allowedIpsText.value = tk.allowed_ips.join(", ");
  dialogOpen.value = true;
}

function num(v: unknown): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = Number(v);
  return Number.isNaN(n) ? null : n;
}

function splitList(v: string): string[] {
  return v
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

async function save() {
  if (!form.name.trim()) {
    error.value = t("trunksPage.nameRequired");
    return;
  }
  saving.value = true;
  error.value = "";
  const payload: TrunkInput = {
    name: form.name.trim(),
    display_name: form.display_name || null,
    carrier: form.carrier || null,
    direction: form.direction,
    sip_server: form.sip_server || null,
    sip_transport: form.sip_transport,
    outbound_proxy: form.outbound_proxy || null,
    auth_username: form.auth_username || null,
    auth_password: form.auth_password || null,
    max_cps: num(form.max_cps),
    max_concurrent: num(form.max_concurrent),
    allowed_ips: splitList(allowedIpsText.value),
    did_numbers: splitList(didsText.value),
    incoming_from_user_prefix: form.incoming_from_user_prefix || null,
    incoming_to_user_prefix: form.incoming_to_user_prefix || null,
    is_active: form.is_active,
    register_enabled: form.register_enabled,
    register_expires: num(form.register_expires),
    rewrite_hostport: form.rewrite_hostport,
  };
  try {
    if (editingId.value) await api.post(`/trunks/${editingId.value}`, payload);
    else await api.post(`/trunks${scope()}`, payload);
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(tk: Trunk) {
  if (!confirm(t("trunksPage.deleteConfirm", { name: tk.name }))) return;
  try {
    await api.del(`/trunks/${tk.id}`);
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
        <h2 class="text-2xl font-bold tracking-tight">{{ t("nav.trunks") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("trunksPage.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="auth.can('trunks:write')" @click="openCreate">
          <Plus class="size-4" />
          {{ t("trunksPage.newTrunk") }}
        </Button>
      </div>
    </div>

    <p v-if="error && !dialogOpen" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead class="w-12">{{ t("common.id") }}</TableHead>
            <TableHead>{{ t("common.name") }}</TableHead>
            <TableHead>{{ t("trunksPage.dest") }}</TableHead>
            <TableHead>{{ t("trunksPage.transport") }}</TableHead>
            <TableHead>{{ t("trunksPage.direction") }}</TableHead>
            <TableHead>{{ t("trunksPage.auth") }}</TableHead>
            <TableHead>{{ t("trunksPage.dids") }}</TableHead>
            <TableHead>{{ t("trunksPage.capacity") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead v-if="auth.can('trunks:write')" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="auth.can('trunks:write') ? 10 : 9">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="trunks.length === 0" :colspan="auth.can('trunks:write') ? 10 : 9">{{ t("trunksPage.noTrunks") }}</TableEmpty>
          <TableRow v-for="tk in trunks" :key="tk.id">
            <TableCell class="text-muted-foreground">{{ tk.id }}</TableCell>
            <TableCell class="font-medium">{{ tk.name }}</TableCell>
            <TableCell class="font-mono text-xs">{{ tk.dest ?? "—" }}</TableCell>
            <TableCell class="uppercase">{{ tk.transport }}</TableCell>
            <TableCell>{{ tk.direction }}</TableCell>
            <TableCell>
              <Badge v-if="tk.has_auth" variant="secondary">✓</Badge>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell>{{ tk.did_numbers.length || "—" }}</TableCell>
            <TableCell>{{ tk.max_concurrent ?? t("common.unlimited") }}</TableCell>
            <TableCell>
              <Badge :variant="tk.is_active ? 'success' : 'muted'">
                {{ tk.is_active ? t("trunksPage.active") : t("trunksPage.inactive") }}
              </Badge>
            </TableCell>
            <TableCell v-if="auth.can('trunks:write')" class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(tk)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(tk)" :aria-label="t('common.delete')">
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
      :title="editingId ? t('trunksPage.editTrunk') : t('trunksPage.newTrunk')"
    >
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="tk-name">{{ t("common.name") }}</Label>
            <Input id="tk-name" v-model="form.name" :class="{ 'border-destructive': !form.name.trim() && form.name.length > 0 }" />
          </div>
          <div class="grid gap-2">
            <Label for="tk-carrier">{{ t("common.optional") }}</Label>
            <Input id="tk-carrier" v-model="form.carrier" placeholder="carrier" />
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="tk-server">{{ t("trunksPage.sipServer") }}</Label>
            <Input id="tk-server" v-model="form.sip_server" />
          </div>
          <div class="grid gap-2">
            <Label for="tk-proxy">{{ t("trunksPage.outboundProxy") }}</Label>
            <Input id="tk-proxy" v-model="form.outbound_proxy" />
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="tk-transport">{{ t("trunksPage.transport") }}</Label>
            <select
              id="tk-transport"
              v-model="form.sip_transport"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="udp">udp</option>
              <option value="tcp">tcp</option>
              <option value="tls">tls</option>
              <option value="ws">ws</option>
              <option value="wss">wss</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="tk-direction">{{ t("trunksPage.direction") }}</Label>
            <select
              id="tk-direction"
              v-model="form.direction"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="inbound">inbound</option>
              <option value="outbound">outbound</option>
              <option value="bidirectional">bidirectional</option>
            </select>
          </div>
        </div>

        <div class="grid gap-2">
          <Label>{{ t("trunksPage.auth") }}</Label>
          <div class="grid grid-cols-2 gap-3">
            <Input v-model="form.auth_username" :placeholder="t('trunksPage.authUsername')" />
            <Input v-model="form.auth_password" type="password" :placeholder="t('trunksPage.authPassword')" autocomplete="new-password" />
          </div>
          <p class="text-xs text-muted-foreground">{{ t("common.optional") }}</p>
        </div>

        <div class="grid gap-2">
          <Label for="tk-dids">{{ t("trunksPage.dids") }}</Label>
          <Input id="tk-dids" v-model="didsText" />
          <p class="text-xs text-muted-foreground">{{ t("trunksPage.didsHint") }}</p>
        </div>

        <div class="grid gap-2">
          <Label for="tk-ips">{{ t("trunksPage.allowedIps") }}</Label>
          <Input id="tk-ips" v-model="allowedIpsText" />
          <p class="text-xs text-muted-foreground">{{ t("trunksPage.allowedIpsHint") }}</p>
        </div>

        <div class="grid grid-cols-3 gap-3">
          <div class="grid gap-2">
            <Label for="tk-cps">{{ t("trunksPage.maxCps") }}</Label>
            <Input id="tk-cps" v-model="form.max_cps" type="number" />
          </div>
          <div class="grid gap-2">
            <Label for="tk-mc">{{ t("trunksPage.capacity") }}</Label>
            <Input id="tk-mc" v-model="form.max_concurrent" type="number" />
          </div>
          <div class="grid gap-2">
            <Label for="tk-exp">{{ t("trunksPage.register") }}</Label>
            <Input id="tk-exp" v-model="form.register_expires" type="number" />
          </div>
        </div>

        <div class="flex flex-wrap gap-x-6 gap-y-2">
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.is_active" class="size-4 rounded border-input" />
            {{ t("trunksPage.isActive") }}
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.register_enabled" class="size-4 rounded border-input" />
            {{ t("trunksPage.registerEnabled") }}
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" v-model="form.rewrite_hostport" class="size-4 rounded border-input" />
            {{ t("trunksPage.rewriteHostport") }}
          </label>
        </div>
      </form>

      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving || !form.name.trim()" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
