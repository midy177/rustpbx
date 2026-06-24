<script setup lang="ts">
import { ref, reactive, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { api, type PlatformSettings, type RecordingPolicy } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Save } from "lucide-vue-next";

const { t } = useI18n();

const baseDomain = ref("");
const stunText = ref(""); // one "host:port" per line
const loading = ref(true);
const saving = ref(false);
const error = ref("");
const savedOk = ref(false);

const rec = reactive<RecordingPolicy>({
  enabled: false,
  type: "local",
  directions: ["inbound", "outbound"],
  auto_start: true,
  path: "",
  samplerate: 8000,
  filename_pattern: null,
  url: null,
  force_file: false,
});
const recEnabled = ref(false);

function parseStun(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function toggleDir(dir: string) {
  const arr = rec.directions ?? [];
  rec.directions = arr.includes(dir) ? arr.filter((d) => d !== dir) : [...arr, dir];
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const s = await api.get<PlatformSettings>("/platform/settings");
    baseDomain.value = s.base_domain;
    stunText.value = (s.stun_servers ?? []).join("\n");
    if (s.recording_policy && s.recording_policy.enabled) {
      Object.assign(rec, s.recording_policy);
      recEnabled.value = true;
    }
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
    const recording_policy: RecordingPolicy | null = recEnabled.value
      ? { ...rec, enabled: true }
      : null;
    const s = await api.put<PlatformSettings>("/platform/settings", {
      base_domain: baseDomain.value.trim(),
      stun_servers: parseStun(stunText.value),
      recording_policy,
    });
    baseDomain.value = s.base_domain;
    stunText.value = (s.stun_servers ?? []).join("\n");
    if (s.recording_policy && s.recording_policy.enabled) {
      Object.assign(rec, s.recording_policy);
      recEnabled.value = true;
    } else {
      recEnabled.value = false;
    }
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
        <CardTitle>{{ t("platform.title") }}</CardTitle>
        <CardDescription>{{ t("platform.subtitle") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-5">
        <div class="grid gap-2">
          <Label for="base-domain">{{ t("platform.baseDomain") }}</Label>
          <Input
            id="base-domain"
            v-model="baseDomain"
            :placeholder="t('platform.baseDomainPlaceholder')"
            :disabled="loading"
          />
          <p class="text-xs text-muted-foreground">{{ t("platform.baseDomainHint") }}</p>
        </div>

        <div class="grid gap-2">
          <Label for="stun">{{ t("platform.stun") }}</Label>
          <textarea
            id="stun"
            v-model="stunText"
            :disabled="loading"
            rows="4"
            :placeholder="t('platform.stunPlaceholder')"
            class="rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          ></textarea>
          <p class="text-xs text-muted-foreground">{{ t("platform.stunHint") }}</p>
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

    <!-- Global recording policy -->
    <Card class="max-w-2xl">
      <CardHeader>
        <CardTitle>{{ t("recording.title") }}</CardTitle>
        <CardDescription>{{ t("recording.subtitle") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-5">
        <label class="flex items-center gap-2 text-sm">
          <input type="checkbox" class="size-4" v-model="recEnabled" />
          {{ t("recording.enable") }}
        </label>

        <div v-if="recEnabled" class="space-y-4">
          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-2">
              <Label>{{ t("recording.type") }}</Label>
              <select v-model="rec.type" class="h-9 rounded-md border border-input bg-transparent px-3 text-sm">
                <option value="local">{{ t("recording.typeLocal") }}</option>
                <option value="http">{{ t("recording.typeHttp") }}</option>
                <option value="s3">{{ t("recording.typeS3") }}</option>
              </select>
            </div>
            <div class="grid gap-2">
              <Label>{{ t("recording.samplerate") }}</Label>
              <Input v-model.number="rec.samplerate" type="number" min="8000" />
            </div>
          </div>

          <div class="grid gap-2">
            <Label>{{ t("recording.directions") }}</Label>
            <div class="flex gap-4 text-sm">
              <label class="flex items-center gap-1"><input type="checkbox" class="size-4" :checked="rec.directions?.includes('inbound')" @change="toggleDir('inbound')" /> inbound</label>
              <label class="flex items-center gap-1"><input type="checkbox" class="size-4" :checked="rec.directions?.includes('outbound')" @change="toggleDir('outbound')" /> outbound</label>
              <label class="flex items-center gap-1"><input type="checkbox" class="size-4" :checked="rec.directions?.includes('internal')" @change="toggleDir('internal')" /> internal</label>
            </div>
          </div>

          <div v-if="rec.type === 'local'" class="grid gap-2">
            <Label>{{ t("recording.path") }}</Label>
            <Input v-model="rec.path" placeholder="/var/lib/rustpbx/recordings" />
          </div>
          <div v-else-if="rec.type === 'http'" class="grid gap-2">
            <Label>{{ t("recording.uploadUrl") }}</Label>
            <Input v-model="rec.url" placeholder="https://upload.example.com/recordings" />
          </div>
          <div v-else class="grid grid-cols-2 gap-3">
            <div class="grid gap-2"><Label>{{ t("recording.bucket") }}</Label><Input v-model="rec.bucket" /></div>
            <div class="grid gap-2"><Label>{{ t("recording.region") }}</Label><Input v-model="rec.region" /></div>
            <div class="grid gap-2 col-span-2"><Label>{{ t("recording.endpoint") }}</Label><Input v-model="rec.endpoint" /></div>
          </div>

          <div class="grid grid-cols-2 gap-3">
            <div class="grid gap-2">
              <Label>{{ t("recording.filenamePattern") }}</Label>
              <Input v-model="rec.filename_pattern" placeholder="{call_id}.wav" />
            </div>
            <div class="flex items-end">
              <label class="flex items-center gap-2 text-sm">
                <input type="checkbox" class="size-4" v-model="rec.force_file" />
                {{ t("recording.forceFile") }}
              </label>
            </div>
          </div>
          <p class="text-xs text-muted-foreground">{{ t("recording.hint") }}</p>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
