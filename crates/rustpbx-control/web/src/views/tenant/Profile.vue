<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Tenant } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

const { t } = useI18n();
const auth = useAuthStore();
const tenant = ref<Tenant | null>(null);

// Self-service password change (POST /me/password — platform admin rejected).
const currentPw = ref("");
const newPw = ref("");
const confirmPw = ref("");
const pwBusy = ref(false);
const pwMsg = ref("");
const pwErr = ref("");

async function changePassword() {
  pwErr.value = "";
  pwMsg.value = "";
  if (newPw.value.length < 6) {
    pwErr.value = t("tenantArea.passwordMinLength");
    return;
  }
  if (newPw.value !== confirmPw.value) {
    pwErr.value = t("tenantArea.passwordMismatch");
    return;
  }
  pwBusy.value = true;
  try {
    await api.post("/me/password", {
      current_password: currentPw.value,
      new_password: newPw.value,
    });
    pwMsg.value = t("tenantArea.passwordChanged");
    currentPw.value = "";
    newPw.value = "";
    confirmPw.value = "";
  } catch (e) {
    pwErr.value = e instanceof Error ? e.message : String(e);
  } finally {
    pwBusy.value = false;
  }
}

onMounted(async () => {
  if (auth.activeTenantId) {
    tenant.value = await api.get<Tenant>(`/tenants/${auth.activeTenantId}`).catch(() => null);
  }
});
</script>

<template>
  <div class="space-y-6">
    <h2 class="text-2xl font-bold tracking-tight">{{ t("tenantArea.profileTitle") }}</h2>
    <Card v-if="tenant" class="max-w-xl">
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          {{ tenant.name }}
          <Badge variant="success">{{ tenant.status }}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent class="space-y-2 text-sm">
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("common.id") }}</span><span>{{ tenant.id }}</span>
        </div>
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("tenants.maxConcurrentCalls") }}</span>
          <span>{{ tenant.max_concurrent_calls ?? t("common.unlimited") }}</span>
        </div>
        <div class="flex justify-between border-b py-2">
          <span class="text-muted-foreground">{{ t("tenants.storagePrefix") }}</span>
          <span>{{ tenant.storage_prefix ?? "—" }}</span>
        </div>
      </CardContent>
    </Card>

    <!-- Self-service password change -->
    <Card class="max-w-xl">
      <CardHeader>
        <CardTitle>{{ t("tenantArea.changePassword") }}</CardTitle>
      </CardHeader>
      <CardContent>
        <form class="space-y-4" @submit.prevent="changePassword">
          <div>
            <label class="mb-1 block text-xs text-muted-foreground">{{ t("tenantArea.currentPassword") }}</label>
            <Input v-model="currentPw" type="password" autocomplete="current-password" />
          </div>
          <div>
            <label class="mb-1 block text-xs text-muted-foreground">{{ t("tenantArea.newPassword") }}</label>
            <Input v-model="newPw" type="password" autocomplete="new-password" />
          </div>
          <div>
            <label class="mb-1 block text-xs text-muted-foreground">{{ t("tenantArea.confirmPassword") }}</label>
            <Input v-model="confirmPw" type="password" autocomplete="new-password" />
          </div>
          <p v-if="pwErr" class="text-sm text-destructive">{{ pwErr }}</p>
          <p v-if="pwMsg" class="text-sm text-primary">{{ pwMsg }}</p>
          <Button type="submit" :disabled="pwBusy || !currentPw || !newPw">
            {{ t("tenantArea.change") }}
          </Button>
        </form>
      </CardContent>
    </Card>
  </div>
</template>
