<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Activity, ExternalLink, PhoneCall, RadioTower, RefreshCw, Settings } from "lucide-vue-next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface PhoneConfig {
  wsPath: string;
  iceServersPath: string;
  amiPath: string;
  staticPath: string;
}

type LoadState = "idle" | "loading" | "ready" | "error";

const phoneConfig = ref<PhoneConfig | null>(null);
const health = ref<Record<string, unknown> | null>(null);
const configState = ref<LoadState>("idle");
const healthState = ref<LoadState>("idle");
const configError = ref("");
const healthError = ref("");

const amiPath = computed(() => phoneConfig.value?.amiPath ?? "/ami/v1");

async function loadJson<T>(path: string): Promise<T> {
  const response = await fetch(path, { credentials: "include" });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`.trim());
  }
  return response.json() as Promise<T>;
}

async function loadConfig() {
  configState.value = "loading";
  configError.value = "";
  try {
    phoneConfig.value = await loadJson<PhoneConfig>("/api/config/phone");
    configState.value = "ready";
  } catch (err) {
    configError.value = err instanceof Error ? err.message : "Failed to load config";
    configState.value = "error";
  }
}

async function loadHealth() {
  healthState.value = "loading";
  healthError.value = "";
  try {
    health.value = await loadJson<Record<string, unknown>>(`${amiPath.value}/health`);
    healthState.value = "ready";
  } catch (err) {
    healthError.value = err instanceof Error ? err.message : "Failed to load AMI health";
    healthState.value = "error";
  }
}

async function refresh() {
  await loadConfig();
  await loadHealth();
}

onMounted(refresh);
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <header class="border-b bg-card">
      <div class="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <div class="flex items-center gap-3">
          <div class="flex h-10 w-10 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <PhoneCall class="h-5 w-5" />
          </div>
          <div>
            <h1 class="text-lg font-semibold">CloudPBX Standalone</h1>
            <p class="text-sm text-muted-foreground">Embedded console served by crates/cloudpbx</p>
          </div>
        </div>
        <Button variant="outline" size="sm" :disabled="configState === 'loading' || healthState === 'loading'" @click="refresh">
          <RefreshCw class="h-4 w-4" />
          Refresh
        </Button>
      </div>
    </header>

    <main class="mx-auto max-w-7xl space-y-6 px-6 py-6">
      <section class="grid gap-4 md:grid-cols-3">
        <Card>
          <CardHeader class="flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle class="text-sm font-medium">Web source</CardTitle>
            <Settings class="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div class="text-2xl font-semibold">crates/cloudpbx/web</div>
            <p class="mt-1 text-sm text-muted-foreground">Built and embedded at Cargo build time.</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader class="flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle class="text-sm font-medium">Runtime config</CardTitle>
            <Activity class="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <Badge :variant="configState === 'ready' ? 'default' : 'outline'">
              {{ configState }}
            </Badge>
            <p v-if="configError" class="mt-2 text-sm text-destructive">{{ configError }}</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader class="flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle class="text-sm font-medium">AMI health</CardTitle>
            <RadioTower class="h-4 w-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <Badge :variant="healthState === 'ready' ? 'default' : 'outline'">
              {{ healthState }}
            </Badge>
            <p v-if="healthError" class="mt-2 text-sm text-destructive">{{ healthError }}</p>
          </CardContent>
        </Card>
      </section>

      <section class="grid gap-4 lg:grid-cols-[0.9fr_1.1fr]">
        <Card>
          <CardHeader>
            <CardTitle>Service paths</CardTitle>
          </CardHeader>
          <CardContent class="space-y-3 text-sm">
            <div class="flex items-center justify-between gap-3">
              <span class="text-muted-foreground">AMI API</span>
              <code class="rounded bg-muted px-2 py-1">{{ amiPath }}</code>
            </div>
            <div class="flex items-center justify-between gap-3">
              <span class="text-muted-foreground">WebSocket</span>
              <code class="rounded bg-muted px-2 py-1">{{ phoneConfig?.wsPath ?? "-" }}</code>
            </div>
            <div class="flex items-center justify-between gap-3">
              <span class="text-muted-foreground">ICE servers</span>
              <code class="rounded bg-muted px-2 py-1">{{ phoneConfig?.iceServersPath ?? "-" }}</code>
            </div>
            <div class="flex items-center justify-between gap-3">
              <span class="text-muted-foreground">Static path</span>
              <code class="rounded bg-muted px-2 py-1">{{ phoneConfig?.staticPath ?? "-" }}</code>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>CloudPBX UI mode</CardTitle>
          </CardHeader>
          <CardContent class="space-y-4">
            <p class="text-sm text-muted-foreground">
              Server-rendered RustPBX console pages are disabled in this standalone build. The
              embedded Vue app at /app is the frontend entry point, while runtime JSON endpoints
              remain available for status and configuration integrations.
            </p>
            <div class="flex flex-wrap gap-3">
              <a
                class="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
                :href="`${amiPath}/health`"
              >
                <ExternalLink class="h-4 w-4" />
                AMI health JSON
              </a>
              <a
                class="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-input bg-background px-4 py-2 text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground"
                href="/api/config/phone"
              >
                <ExternalLink class="h-4 w-4" />
                Runtime config JSON
              </a>
            </div>
          </CardContent>
        </Card>
      </section>

      <Card>
        <CardHeader>
          <CardTitle>Health payload</CardTitle>
        </CardHeader>
        <CardContent>
          <pre class="max-h-[360px] overflow-auto rounded-md bg-muted p-4 text-xs">{{ health ?? { status: healthState } }}</pre>
        </CardContent>
      </Card>
    </main>
  </div>
</template>
