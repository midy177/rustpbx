<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Tenant } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Save, Globe } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();

const tenant = ref<Tenant | null>(null);
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const savedOk = ref(false);
const canWrite = auth.can("domain:write");

const form = reactive({
  custom_domain: "",
  custom_domain_enabled: false,
});

function scope() {
  return auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const tn = await api.get<Tenant>(`/tenant-domain${scope()}`);
    tenant.value = tn;
    form.custom_domain = tn.custom_domain ?? "";
    form.custom_domain_enabled = tn.custom_domain_enabled;
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);

async function save() {
  saving.value = true;
  error.value = "";
  savedOk.value = false;
  try {
    const tn = await api.put<Tenant>(`/tenant-domain${scope()}`, {
      custom_domain: form.custom_domain.trim() || null,
      custom_domain_enabled: form.custom_domain_enabled,
    });
    tenant.value = tn;
    form.custom_domain = tn.custom_domain ?? "";
    form.custom_domain_enabled = tn.custom_domain_enabled;
    savedOk.value = true;
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="space-y-6">
    <div>
      <h2 class="text-2xl font-bold tracking-tight">{{ t("domain.title") }}</h2>
      <p class="text-sm text-muted-foreground">{{ t("domain.subtitle") }}</p>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <!-- Active + default domain summary -->
    <Card class="max-w-2xl">
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          <Globe class="size-4" /> {{ t("domain.activeDomain") }}
        </CardTitle>
      </CardHeader>
      <CardContent class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="font-mono text-sm">{{ tenant?.active_domain || t("domain.notConfigured") }}</span>
          <Badge v-if="tenant?.active_domain" variant="success">{{ t("trunksPage.active") }}</Badge>
        </div>
        <div class="text-sm text-muted-foreground">
          <span class="font-medium">{{ t("domain.defaultDomain") }}:</span>
          <span class="ml-1 font-mono">{{ tenant?.default_domain || "—" }}</span>
          <Badge
            v-if="tenant?.default_domain && tenant?.custom_domain_enabled"
            variant="muted"
            class="ml-2"
          >
            {{ t("domain.reserved") }}
          </Badge>
          <p class="mt-1 text-xs">{{ t("domain.defaultDomainHint") }}</p>
        </div>
      </CardContent>
    </Card>

    <!-- Custom domain editor -->
    <Card class="max-w-2xl">
      <CardHeader>
        <CardTitle>{{ t("domain.customDomain") }}</CardTitle>
        <CardDescription>{{ t("domain.useCustomHint") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-4">
        <div class="grid gap-2">
          <Label for="custom-domain">{{ t("domain.customDomain") }}</Label>
          <Input
            id="custom-domain"
            v-model="form.custom_domain"
            :placeholder="t('domain.customDomainPlaceholder')"
            :disabled="!canWrite || loading"
          />
        </div>
        <label class="flex items-center gap-2 text-sm" :class="{ 'opacity-50': !canWrite }">
          <input
            type="checkbox"
            v-model="form.custom_domain_enabled"
            :disabled="!canWrite || loading || !form.custom_domain.trim()"
            class="size-4 rounded border-input"
          />
          {{ t("domain.useCustom") }}
        </label>
        <div v-if="canWrite" class="flex items-center gap-3">
          <Button :disabled="saving || loading" @click="save">
            <Save class="size-4" />
            {{ t("common.save") }}
          </Button>
          <span v-if="savedOk" class="text-sm text-emerald-600">{{ t("domain.savedOk") }}</span>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
