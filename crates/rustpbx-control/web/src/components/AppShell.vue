<script setup lang="ts">
import { ref, onMounted, type Component } from "vue";
import { RouterView, RouterLink, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { useAuthStore } from "@/stores/auth";
import { SUPPORTED_LOCALES, setLocale, type AppLocale } from "@/i18n";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { LogOut, Moon, Sun, Languages } from "lucide-vue-next";

export interface NavItem {
  to: string;
  labelKey: string;
  icon: Component;
}

const props = defineProps<{
  areaTitleKey: string;
  nav: NavItem[];
  /** When scoped to a tenant, show a badge + an "exit" button to this route. */
  scopeLabel?: string;
  exitTo?: string;
}>();

function onExit() {
  if (props.exitTo) {
    auth.setActiveTenant(null);
    router.push(props.exitTo);
  }
}

const { t, locale } = useI18n();
const auth = useAuthStore();
const router = useRouter();

const isDark = ref(false);
onMounted(() => {
  isDark.value = localStorage.getItem("rustpbx.control.theme") === "dark";
  applyTheme();
});
function applyTheme() {
  document.documentElement.classList.toggle("dark", isDark.value);
}
function toggleTheme() {
  isDark.value = !isDark.value;
  localStorage.setItem("rustpbx.control.theme", isDark.value ? "dark" : "light");
  applyTheme();
}

function changeLocale(e: Event) {
  setLocale((e.target as HTMLSelectElement).value as AppLocale);
}

async function onLogout() {
  await auth.logout();
  router.push({ name: "login" });
}
</script>

<template>
  <div class="flex min-h-screen bg-background text-foreground">
    <!-- Sidebar -->
    <aside class="hidden w-60 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground md:flex">
      <div class="flex h-14 items-center gap-2 border-b px-5 font-semibold">
        <span class="text-lg">📞</span>
        <span>{{ t("common.appName") }}</span>
      </div>
      <div class="px-3 py-2 text-xs font-medium uppercase tracking-wider text-muted-foreground">
        {{ t(props.areaTitleKey) }}
      </div>
      <nav class="flex flex-1 flex-col gap-1 px-3">
        <RouterLink
          v-for="item in nav"
          :key="item.to"
          :to="item.to"
          class="flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium text-sidebar-foreground/80 transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
          active-class="bg-sidebar-accent text-sidebar-accent-foreground"
        >
          <component :is="item.icon" class="size-4" />
          {{ t(item.labelKey) }}
        </RouterLink>
      </nav>
      <div class="border-t p-3 text-xs text-muted-foreground">v0.4.4 · RustPBX</div>
    </aside>

    <!-- Main -->
    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex h-14 items-center gap-3 border-b px-4 md:px-6">
        <h1 class="truncate text-sm font-semibold">{{ t(props.areaTitleKey) }}</h1>
        <Badge v-if="scopeLabel" variant="secondary" class="ml-1">{{ scopeLabel }}</Badge>
        <Button v-if="exitTo" variant="ghost" size="sm" class="ml-1" @click="onExit">
          {{ t("nav.exitTenant") }}
        </Button>
        <div class="ml-auto flex items-center gap-2">
          <div class="relative flex items-center">
            <Languages class="pointer-events-none absolute left-2 size-4 text-muted-foreground" />
            <select
              :value="locale"
              @change="changeLocale"
              class="h-8 rounded-md border border-input bg-transparent pl-7 pr-2 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option v-for="l in SUPPORTED_LOCALES" :key="l.value" :value="l.value">{{ l.label }}</option>
            </select>
          </div>
          <Button variant="ghost" size="icon" @click="toggleTheme" :aria-label="t('nav.theme')">
            <Moon v-if="!isDark" />
            <Sun v-else />
          </Button>
          <Badge variant="muted">{{ t(`roles.${auth.user?.role ?? "tenant"}`) }}</Badge>
          <span class="hidden text-sm text-muted-foreground sm:inline">{{ auth.user?.username }}</span>
          <Button variant="outline" size="sm" @click="onLogout">
            <LogOut class="size-4" />
            <span class="hidden sm:inline">{{ t("auth.logout") }}</span>
          </Button>
        </div>
      </header>

      <main class="flex-1 overflow-auto p-4 md:p-6">
        <RouterView />
      </main>
    </div>
  </div>
</template>
