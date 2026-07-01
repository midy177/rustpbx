<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Building2, Cable, GitBranch, LogOut, Phone, RadioTower, Users } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api, type TenantSummary } from "@/api/client";
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
const activeTenant = computed(() => auth.user?.tenant?.name ?? tenants.value[0]?.name ?? "default");

const resources = computed(() => [
  { label: "Extensions", value: String(extensionCount.value), icon: Phone },
  { label: "SIP trunks", value: String(sipTrunkCount.value), icon: Cable },
  { label: "Routes", value: String(routeCount.value), icon: GitBranch },
  { label: "Call records", value: String(callRecordCount.value), icon: RadioTower },
  { label: "Users", value: String(userCount.value), icon: Users },
]);

onMounted(async () => {
  try {
    tenants.value = await api.tenants();
  } catch {
    tenants.value = [{ id: "default", name: "Default", status: "active" }];
  }
  try {
    const [extensions, trunks, routes, callRecords, users] = await Promise.all([
      api.extensions(),
      api.sipTrunks(),
      api.routes(),
      api.callRecords(),
      api.users(),
    ]);
    extensionCount.value = extensions.length;
    sipTrunkCount.value = trunks.length;
    routeCount.value = routes.length;
    callRecordCount.value = callRecords.length;
    userCount.value = users.length;
  } catch {
    extensionCount.value = 0;
    sipTrunkCount.value = 0;
    routeCount.value = 0;
    callRecordCount.value = 0;
    userCount.value = 0;
  }
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
    </main>
  </div>
</template>
