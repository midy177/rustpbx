<script setup lang="ts">
import { ref, reactive, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import {
  api,
  ALL_PERMISSIONS,
  type TenantUser,
  type CreateTenantUser,
  type UpdateTenantUser,
} from "@/api/client";
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
import { Plus, Pencil, Trash2, RefreshCw } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();

const users = ref<TenantUser[]>([]);
const loading = ref(true);
const error = ref("");
const canWrite = auth.can("users:write");
// Only a super-admin may mint another tenant admin.
const canMakeAdmin = auth.isSuperAdmin;

const dialogOpen = ref(false);
const editingId = ref<number | null>(null);
const saving = ref(false);

const form = reactive<{
  username: string;
  display_name: string;
  password: string;
  role: string;
  status: string;
  permissions: string[];
}>({
  username: "",
  display_name: "",
  password: "",
  role: "user",
  status: "active",
  permissions: [],
});

const isAdminRole = computed(() => form.role === "admin");

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    users.value = await api.get<TenantUser[]>(`/tenant-users${scope()}`);
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
    username: "",
    display_name: "",
    password: "",
    role: "user",
    status: "active",
    permissions: [],
  });
  dialogOpen.value = true;
}

function openEdit(u: TenantUser) {
  editingId.value = u.id;
  Object.assign(form, {
    username: u.username,
    display_name: u.display_name ?? "",
    password: "",
    role: u.role,
    status: u.status,
    permissions: [...u.permissions],
  });
  dialogOpen.value = true;
}

function togglePermission(p: string) {
  const i = form.permissions.indexOf(p);
  if (i >= 0) form.permissions.splice(i, 1);
  else form.permissions.push(p);
}

async function save() {
  if (!form.username.trim()) {
    error.value = t("iam.usernameRequired");
    return;
  }
  if (!editingId.value && form.password.length < 6) {
    error.value = t("iam.passwordRequired");
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    if (editingId.value) {
      const payload: UpdateTenantUser = {
        display_name: form.display_name || null,
        role: form.role,
        status: form.status,
        permissions: isAdminRole.value ? [] : form.permissions,
      };
      if (form.password) payload.password = form.password;
      await api.post(`/tenant-users/${editingId.value}`, payload);
    } else {
      const payload: CreateTenantUser = {
        username: form.username.trim(),
        password: form.password,
        display_name: form.display_name || null,
        role: form.role,
        permissions: isAdminRole.value ? [] : form.permissions,
      };
      await api.post(`/tenant-users${scope()}`, payload);
    }
    dialogOpen.value = false;
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}

async function remove(u: TenantUser) {
  if (!confirm(t("iam.deleteConfirm", { name: u.username }))) return;
  try {
    await api.del(`/tenant-users/${u.id}`);
    await load();
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  }
}

function roleLabel(r: string) {
  return r === "admin" ? t("roles.tenant_admin") : t("roles.tenant_user");
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("iam.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("iam.subtitle") }}</p>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
          <RefreshCw class="size-4" />
        </Button>
        <Button v-if="canWrite" @click="openCreate">
          <Plus class="size-4" />
          {{ t("iam.newUser") }}
        </Button>
      </div>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("iam.username") }}</TableHead>
            <TableHead>{{ t("iam.displayName") }}</TableHead>
            <TableHead>{{ t("iam.role") }}</TableHead>
            <TableHead>{{ t("iam.permissions") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("iam.lastLogin") }}</TableHead>
            <TableHead v-if="canWrite" class="text-right">{{ t("common.actions") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="canWrite ? 7 : 6">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="users.length === 0" :colspan="canWrite ? 7 : 6">{{ t("iam.noUsers") }}</TableEmpty>
          <TableRow v-for="u in users" :key="u.id">
            <TableCell class="font-medium">{{ u.username }}</TableCell>
            <TableCell class="text-muted-foreground">{{ u.display_name || "—" }}</TableCell>
            <TableCell>
              <Badge :variant="u.role === 'admin' ? 'secondary' : 'muted'">{{ roleLabel(u.role) }}</Badge>
            </TableCell>
            <TableCell>
              <span v-if="u.role === 'admin'" class="text-xs text-muted-foreground">{{ t("common.unlimited") }}</span>
              <span v-else class="text-xs">{{ u.permissions.length }}</span>
            </TableCell>
            <TableCell>
              <Badge :variant="u.status === 'active' ? 'success' : 'warning'">
                {{ u.status === "active" ? t("iam.statusActive") : t("iam.statusSuspended") }}
              </Badge>
            </TableCell>
            <TableCell class="text-muted-foreground text-xs">
              {{ u.last_login_at ? formatDate(u.last_login_at) : t("iam.never") }}
            </TableCell>
            <TableCell v-if="canWrite" class="text-right">
              <div class="flex justify-end gap-1">
                <Button variant="ghost" size="icon" @click="openEdit(u)" :aria-label="t('common.edit')">
                  <Pencil class="size-4" />
                </Button>
                <Button variant="ghost" size="icon" @click="remove(u)" :aria-label="t('common.delete')">
                  <Trash2 class="size-4 text-destructive" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <Dialog v-model:open="dialogOpen" :title="editingId ? t('iam.editUser') : t('iam.newUser')">
      <form class="grid gap-4" @submit.prevent="save">
        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="u-name">{{ t("iam.username") }}</Label>
            <Input id="u-name" v-model="form.username" :disabled="!!editingId" autocomplete="off" />
          </div>
          <div class="grid gap-2">
            <Label for="u-display">{{ t("iam.displayName") }}</Label>
            <Input id="u-display" v-model="form.display_name" />
          </div>
        </div>

        <div class="grid gap-2">
          <Label for="u-pass">{{ editingId ? t("iam.passwordKeep") : t("iam.password") }}</Label>
          <Input id="u-pass" v-model="form.password" type="password" autocomplete="new-password" />
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="grid gap-2">
            <Label for="u-role">{{ t("iam.role") }}</Label>
            <select
              id="u-role"
              v-model="form.role"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option v-if="canMakeAdmin || form.role === 'admin'" value="admin">{{ t("iam.roleAdmin") }}</option>
              <option value="user">{{ t("iam.roleUser") }}</option>
            </select>
          </div>
          <div class="grid gap-2">
            <Label for="u-status">{{ t("common.status") }}</Label>
            <select
              id="u-status"
              v-model="form.status"
              class="h-9 rounded-md border border-input bg-transparent px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="active">{{ t("iam.statusActive") }}</option>
              <option value="suspended">{{ t("iam.statusSuspended") }}</option>
            </select>
          </div>
        </div>

        <div v-if="!isAdminRole" class="grid gap-2">
          <Label>{{ t("iam.permissions") }}</Label>
          <p class="text-xs text-muted-foreground">{{ t("iam.permissionsHint") }}</p>
          <div class="grid grid-cols-2 gap-1.5 rounded-md border p-3">
            <label v-for="p in ALL_PERMISSIONS" :key="p" class="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                :checked="form.permissions.includes(p)"
                @change="togglePermission(p)"
                class="size-4 rounded border-input"
              />
              <span class="font-mono text-xs">{{ p }}</span>
            </label>
          </div>
        </div>
      </form>

      <template #footer>
        <Button variant="outline" @click="dialogOpen = false">{{ t("common.cancel") }}</Button>
        <Button :disabled="saving" @click="save">{{ t("common.save") }}</Button>
      </template>
    </Dialog>
  </div>
</template>
