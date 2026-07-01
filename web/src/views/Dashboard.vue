<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Building2, Cable, GitBranch, LogOut, Phone, Plus, RadioTower, Trash2, Users } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, type ExtensionSummary, type TenantSummary } from "@/api/client";
import { useAuthStore } from "@/stores/auth";

const auth = useAuthStore();
const router = useRouter();
const tenants = ref<TenantSummary[]>([]);
const extensionCount = ref(0);
const sipTrunkCount = ref(0);
const routeCount = ref(0);
const callRecordCount = ref(0);
const userCount = ref(0);
const signingOut = ref(false);
const extensions = ref<ExtensionSummary[]>([]);
const extensionForm = ref({ extension: "", display_name: "", email: "" });
const extensionError = ref("");
const creatingExtension = ref(false);
const deletingExtensionId = ref<number | null>(null);
const activeTenant = computed(() => auth.user?.tenant?.name ?? tenants.value[0]?.name ?? "default");
const canManage = computed(() => auth.user?.role === "platform_admin" || auth.user?.role === "tenant_admin");

const resources = computed(() => [
  { label: "Extensions", value: String(extensionCount.value), icon: Phone },
  { label: "SIP trunks", value: String(sipTrunkCount.value), icon: Cable },
  { label: "Routes", value: String(routeCount.value), icon: GitBranch },
  { label: "Call records", value: String(callRecordCount.value), icon: RadioTower },
  { label: "Users", value: String(userCount.value), icon: Users },
]);

async function loadDashboard() {
  try {
    tenants.value = await api.tenants();
  } catch {
    tenants.value = [{ id: "default", name: "Default", status: "active" }];
  }
  try {
    const [loadedExtensions, trunks, routes, callRecords, users] = await Promise.all([
      api.extensions(),
      api.sipTrunks(),
      api.routes(),
      api.callRecords(),
      api.users(),
    ]);
    extensions.value = loadedExtensions;
    extensionCount.value = loadedExtensions.length;
    sipTrunkCount.value = trunks.length;
    routeCount.value = routes.length;
    callRecordCount.value = callRecords.length;
    userCount.value = users.length;
  } catch {
    extensions.value = [];
    extensionCount.value = 0;
    sipTrunkCount.value = 0;
    routeCount.value = 0;
    callRecordCount.value = 0;
    userCount.value = 0;
  }
}

onMounted(async () => {
  await loadDashboard();
});

async function signOut() {
  signingOut.value = true;
  try {
    await auth.logout();
    await router.push({ name: "login" });
  } finally {
    signingOut.value = false;
  }
}

async function createExtension() {
  extensionError.value = "";
  const extension = extensionForm.value.extension.trim();
  if (!extension) {
    extensionError.value = "Extension is required";
    return;
  }

  creatingExtension.value = true;
  try {
    const created = await api.createExtension({
      extension,
      display_name: extensionForm.value.display_name.trim() || null,
      email: extensionForm.value.email.trim() || null,
      status: "active",
    });
    extensions.value = [created, ...extensions.value].sort((a, b) => a.extension.localeCompare(b.extension));
    extensionCount.value = extensions.value.length;
    extensionForm.value = { extension: "", display_name: "", email: "" };
  } catch (err) {
    extensionError.value = err instanceof Error ? err.message : "Failed to create extension";
  } finally {
    creatingExtension.value = false;
  }
}

async function deleteExtension(extension: ExtensionSummary) {
  extensionError.value = "";
  deletingExtensionId.value = extension.id;
  try {
    await api.deleteExtension(extension.id);
    extensions.value = extensions.value.filter((item) => item.id !== extension.id);
    extensionCount.value = extensions.value.length;
  } catch (err) {
    extensionError.value = err instanceof Error ? err.message : "Failed to delete extension";
  } finally {
    deletingExtensionId.value = null;
  }
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <header class="border-b bg-card">
      <div class="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <div class="flex items-center gap-3">
          <div class="flex h-9 w-9 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Building2 class="h-5 w-5" />
          </div>
          <div>
            <h1 class="text-lg font-semibold">CloudPBX</h1>
            <p class="text-sm text-muted-foreground">Monolith multi-tenant console</p>
          </div>
        </div>
        <div class="flex items-center gap-3">
          <Badge variant="secondary">{{ activeTenant }}</Badge>
          <Button variant="outline" size="sm" :disabled="signingOut" @click="signOut">
            <LogOut class="h-4 w-4" />
            Sign out
          </Button>
        </div>
      </div>
    </header>

    <main class="mx-auto max-w-7xl space-y-6 px-6 py-6">
      <section class="grid gap-4 md:grid-cols-2 xl:grid-cols-5">
        <Card v-for="item in resources" :key="item.label">
          <CardHeader class="flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle class="text-sm font-medium">{{ item.label }}</CardTitle>
            <component :is="item.icon" class="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div class="text-2xl font-semibold">{{ item.value }}</div>
          </CardContent>
        </Card>
      </section>

      <section class="grid gap-4 lg:grid-cols-[1.2fr_0.8fr]">
        <Card>
          <CardHeader>
            <CardTitle>Tenant scope</CardTitle>
          </CardHeader>
          <CardContent class="space-y-3">
            <div v-for="tenant in tenants" :key="tenant.id" class="flex items-center justify-between rounded-md border px-3 py-2">
              <div>
                <div class="font-medium">{{ tenant.name }}</div>
                <div class="text-sm text-muted-foreground">{{ tenant.domain ?? tenant.id }}</div>
              </div>
              <Badge :variant="tenant.status === 'active' ? 'default' : 'outline'">{{ tenant.status }}</Badge>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Migration status</CardTitle>
          </CardHeader>
          <CardContent class="space-y-3 text-sm">
            <div class="flex items-center justify-between">
              <span>SPA shell</span>
              <Badge>ready</Badge>
            </div>
            <div class="flex items-center justify-between">
              <span>Tenant context API</span>
              <Badge>ready</Badge>
            </div>
            <div class="flex items-center justify-between">
              <span>Tenant-aware read APIs</span>
              <Badge>ready</Badge>
            </div>
          </CardContent>
        </Card>
      </section>

      <section>
        <Card>
          <CardHeader>
            <CardTitle>Extensions</CardTitle>
          </CardHeader>
          <CardContent class="space-y-4">
            <form v-if="canManage" class="grid gap-3 md:grid-cols-[140px_1fr_1fr_auto]" @submit.prevent="createExtension">
              <div class="space-y-2">
                <Label for="new-extension">Extension</Label>
                <Input id="new-extension" v-model="extensionForm.extension" autocomplete="off" />
              </div>
              <div class="space-y-2">
                <Label for="new-extension-name">Display name</Label>
                <Input id="new-extension-name" v-model="extensionForm.display_name" autocomplete="off" />
              </div>
              <div class="space-y-2">
                <Label for="new-extension-email">Email</Label>
                <Input id="new-extension-email" v-model="extensionForm.email" autocomplete="off" />
              </div>
              <div class="flex items-end">
                <Button type="submit" :disabled="creatingExtension">
                  <Plus class="h-4 w-4" />
                  Add
                </Button>
              </div>
            </form>
            <p v-if="extensionError" class="text-sm text-destructive">{{ extensionError }}</p>

            <div class="overflow-x-auto rounded-md border">
              <table class="w-full min-w-[680px] text-left text-sm">
                <thead class="bg-muted/50 text-muted-foreground">
                  <tr>
                    <th class="px-3 py-2 font-medium">Extension</th>
                    <th class="px-3 py-2 font-medium">Display name</th>
                    <th class="px-3 py-2 font-medium">Email</th>
                    <th class="px-3 py-2 font-medium">Status</th>
                    <th class="px-3 py-2 text-right font-medium">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="extension in extensions" :key="extension.id" class="border-t">
                    <td class="px-3 py-2 font-medium">{{ extension.extension }}</td>
                    <td class="px-3 py-2">{{ extension.display_name || "-" }}</td>
                    <td class="px-3 py-2">{{ extension.email || "-" }}</td>
                    <td class="px-3 py-2">
                      <Badge :variant="extension.status === 'active' ? 'default' : 'outline'">
                        {{ extension.status ?? "unknown" }}
                      </Badge>
                    </td>
                    <td class="px-3 py-2 text-right">
                      <Button
                        v-if="canManage"
                        variant="ghost"
                        size="icon"
                        :disabled="deletingExtensionId === extension.id"
                        aria-label="Delete extension"
                        @click="deleteExtension(extension)"
                      >
                        <Trash2 class="h-4 w-4" />
                      </Button>
                    </td>
                  </tr>
                  <tr v-if="extensions.length === 0">
                    <td class="px-3 py-6 text-center text-muted-foreground" colspan="5">No extensions</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      </section>
    </main>
  </div>
</template>
