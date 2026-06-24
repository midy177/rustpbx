<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type CallRecord, type CdrPage } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { formatDate } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell, TableEmpty,
} from "@/components/ui/table";
import { RefreshCw, Search, Download } from "lucide-vue-next";

const { t } = useI18n();
const auth = useAuthStore();
const records = ref<CallRecord[]>([]);
const total = ref(0);
const loading = ref(true);
const error = ref("");

const PAGE_SIZE = 50;
const offset = ref(0);

// Filters (applied on submit, not on every keystroke).
const search = ref("");
const status = ref("");
const direction = ref("");
const since = ref(""); // yyyy-mm-dd
const until = ref("");

const rangeFrom = computed(() => (total.value === 0 ? 0 : offset.value + 1));
const rangeTo = computed(() => Math.min(offset.value + records.value.length, total.value));
const canPrev = computed(() => offset.value > 0);
const canNext = computed(() => offset.value + PAGE_SIZE < total.value);

function fmtDuration(secs: number) {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function statusVariant(s: string) {
  const v = s.toLowerCase();
  if (v.includes("complet") || v.includes("answer")) return "success" as const;
  if (v.includes("fail") || v.includes("reject") || v.includes("error")) return "warning" as const;
  return "muted" as const;
}

/** Convert a yyyy-mm-dd input into an RFC3339 bound (until → end of day). */
function toRfc3339(date: string, endOfDay: boolean): string | undefined {
  if (!date) return undefined;
  return `${date}T${endOfDay ? "23:59:59" : "00:00:00"}Z`;
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const p = new URLSearchParams();
    if (auth.activeTenantId) p.set("tenant_id", String(auth.activeTenantId));
    p.set("limit", String(PAGE_SIZE));
    p.set("offset", String(offset.value));
    if (search.value.trim()) p.set("search", search.value.trim());
    if (status.value) p.set("status", status.value);
    if (direction.value) p.set("direction", direction.value);
    const s = toRfc3339(since.value, false);
    const u = toRfc3339(until.value, true);
    if (s) p.set("since", s);
    if (u) p.set("until", u);
    const page = await api.get<CdrPage>(`/call-records?${p.toString()}`);
    records.value = page.records;
    total.value = page.total;
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}

function applyFilters() {
  offset.value = 0;
  load();
}

function resetFilters() {
  search.value = "";
  status.value = "";
  direction.value = "";
  since.value = "";
  until.value = "";
  applyFilters();
}

function prevPage() {
  if (!canPrev.value) return;
  offset.value = Math.max(0, offset.value - PAGE_SIZE);
  load();
}

function nextPage() {
  if (!canNext.value) return;
  offset.value += PAGE_SIZE;
  load();
}

onMounted(load);
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between gap-4">
      <div>
        <h2 class="text-2xl font-bold tracking-tight">{{ t("cdrPage.title") }}</h2>
        <p class="text-sm text-muted-foreground">{{ t("cdrPage.subtitle") }}</p>
      </div>
      <Button variant="outline" size="icon" @click="load" :aria-label="t('common.refresh')">
        <RefreshCw class="size-4" />
      </Button>
    </div>

    <Card class="p-4">
      <form class="flex flex-wrap items-end gap-3" @submit.prevent="applyFilters">
        <div class="flex-1 min-w-[200px]">
          <Input
            v-model="search"
            :placeholder="t('cdrPage.searchPlaceholder')"
            class="w-full"
          />
        </div>
        <select
          v-model="status"
          class="h-9 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm"
        >
          <option value="">{{ t("cdrPage.anyStatus") }}</option>
          <option value="completed">completed</option>
          <option value="answered">answered</option>
          <option value="no_answer">no_answer</option>
          <option value="busy">busy</option>
          <option value="failed">failed</option>
        </select>
        <select
          v-model="direction"
          class="h-9 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm"
        >
          <option value="">{{ t("cdrPage.anyDirection") }}</option>
          <option value="inbound">{{ t("cdrPage.inbound") }}</option>
          <option value="outbound">{{ t("cdrPage.outbound") }}</option>
        </select>
        <Input v-model="since" type="date" :aria-label="t('cdrPage.since')" class="w-auto" />
        <Input v-model="until" type="date" :aria-label="t('cdrPage.until')" class="w-auto" />
        <Button type="submit">
          <Search class="size-4" /> {{ t("cdrPage.apply") }}
        </Button>
        <Button type="button" variant="outline" @click="resetFilters">{{ t("cdrPage.reset") }}</Button>
      </form>
    </Card>

    <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

    <Card>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t("cdrPage.from") }}</TableHead>
            <TableHead>{{ t("cdrPage.to") }}</TableHead>
            <TableHead>{{ t("cdrPage.direction") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("cdrPage.duration") }}</TableHead>
            <TableHead>{{ t("cdrPage.started") }}</TableHead>
            <TableHead>{{ t("cdrPage.recording") }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableEmpty v-if="loading" :colspan="7">{{ t("common.loading") }}</TableEmpty>
          <TableEmpty v-else-if="records.length === 0" :colspan="7">{{ t("cdrPage.noRecords") }}</TableEmpty>
          <TableRow v-for="r in records" :key="r.id">
            <TableCell class="font-mono text-xs">{{ r.from_number ?? "—" }}</TableCell>
            <TableCell class="font-mono text-xs">{{ r.to_number ?? "—" }}</TableCell>
            <TableCell>{{ r.direction }}</TableCell>
            <TableCell><Badge :variant="statusVariant(r.status)">{{ r.status }}</Badge></TableCell>
            <TableCell>{{ fmtDuration(r.duration_secs) }}</TableCell>
            <TableCell class="text-muted-foreground text-xs">
              {{ r.started_at ? formatDate(r.started_at) : "—" }}
            </TableCell>
            <TableCell>
              <div v-if="r.recording_url" class="flex items-center gap-2">
                <audio :src="r.recording_url" controls preload="none" class="h-8 max-w-[200px]" />
                <a
                  :href="r.recording_url"
                  download
                  class="text-muted-foreground hover:text-primary"
                  :aria-label="t('cdrPage.download')"
                  :title="t('cdrPage.download')"
                >
                  <Download class="size-4" />
                </a>
              </div>
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </Card>

    <div v-if="total > 0" class="flex items-center justify-between text-sm text-muted-foreground">
      <span>{{ t("cdrPage.showing", { from: rangeFrom, to: rangeTo, total }) }}</span>
      <div class="flex gap-2">
        <Button variant="outline" size="sm" :disabled="!canPrev" @click="prevPage">
          {{ t("cdrPage.prev") }}
        </Button>
        <Button variant="outline" size="sm" :disabled="!canNext" @click="nextPage">
          {{ t("cdrPage.next") }}
        </Button>
      </div>
    </div>
  </div>
</template>
