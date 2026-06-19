<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type Trunk } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw, Lock } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const trunks = ref<Trunk[]>([]);
const loading = ref(true);
const error = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const q = auth.activeTenantId ? `?tenant_id=${auth.activeTenantId}` : "";
    trunks.value = await api.get<Trunk[]>(`/trunks${q}`);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
onMounted(load);
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("nav.trunks") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("trunksPage.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <p class="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Lock class="size-3" /> {{ t("trunksPage.readonly") }}
    </p>
    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

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
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="9">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="trunks.length === 0" :colspan="9">{{ t("trunksPage.noTrunks") }}</TableEmpty>
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
          </TableRow>
        </TableBody>
      </Table>
    </Card>
  </div>
</template>
