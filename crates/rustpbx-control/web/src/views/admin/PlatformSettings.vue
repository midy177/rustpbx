<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type PlatformSettings } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Save } from "lucide-vue-next";

const { t } = useI18n();

const baseDomain = ref("");
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const savedOk = ref(false);

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const s = await api.get<PlatformSettings>("/platform/settings");
    baseDomain.value = s.base_domain;
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
    const s = await api.put<PlatformSettings>("/platform/settings", {
      base_domain: baseDomain.value.trim(),
    });
    baseDomain.value = s.base_domain;
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
      <h2 class="text-2xl font-bold tracking-tight">{{ t("platform.title") }}</h2>
      <p class="text-sm text-muted-foreground">{{ t("platform.subtitle") }}</p>
    </div>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card class="max-w-2xl">
      <CardHeader>
        <CardTitle>{{ t("platform.baseDomain") }}</CardTitle>
        <CardDescription>{{ t("platform.baseDomainHint") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-4">
        <div class="grid gap-2">
          <Label for="base-domain">{{ t("platform.baseDomain") }}</Label>
          <Input
            id="base-domain"
            v-model="baseDomain"
            :placeholder="t('platform.baseDomainPlaceholder')"
            :disabled="loading"
          />
        </div>
        <div class="flex items-center gap-3">
          <Button :disabled="saving || loading" @click="save">
            <Save class="size-4" />
            {{ t("common.save") }}
          </Button>
          <span v-if="savedOk" class="text-sm text-emerald-600">{{ t("platform.savedOk") }}</span>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
