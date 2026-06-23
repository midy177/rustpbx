<script setup lang="ts">
import { ref, computed } from "vue";
import { useRouter, useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { useAuthStore } from "@/stores/auth";
import { SUPPORTED_LOCALES, setLocale, type AppLocale } from "@/i18n";
import { ApiError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { ShieldCheck, Building2 } from "lucide-vue-next";

const { t, locale } = useI18n();
const auth = useAuthStore();
const router = useRouter();
const route = useRoute();

// "iam" (tenant account, default) or "root" (platform super-admin), like a
// cloud console that defaults to IAM sign-in with a link to the root user.
const mode = ref<"iam" | "root">("iam");

const username = ref("");
const password = ref("");
const domain = ref("");
const error = ref("");
const loading = ref(false);

const title = computed(() => (mode.value === "iam" ? t("auth.iamTitle") : t("auth.rootTitle")));
const subtitle = computed(() =>
  mode.value === "iam" ? t("auth.iamSubtitle") : t("auth.rootSubtitle"),
);

function switchMode(to: "iam" | "root") {
  mode.value = to;
  error.value = "";
  password.value = "";
}

function changeLocale(e: Event) {
  setLocale((e.target as HTMLSelectElement).value as AppLocale);
}

async function onSubmit() {
  error.value = "";
  if (mode.value === "iam" && !domain.value.trim()) {
    error.value = t("auth.domainRequired");
    return;
  }
  loading.value = true;
  try {
    // Root mode never sends a domain; IAM mode always does.
    await auth.login(username.value, password.value, mode.value === "iam" ? domain.value : undefined);
    const redirect = (route.query.redirect as string) || auth.homeRoute();
    router.push(redirect);
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) error.value = t("auth.invalidCredentials");
    else error.value = e instanceof Error ? e.message : t("auth.loginFailed");
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <div class="flex min-h-screen items-center justify-center bg-muted/40 p-4">
    <div class="absolute right-4 top-4">
      <select
        :value="locale"
        @change="changeLocale"
        class="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
      >
        <option v-for="l in SUPPORTED_LOCALES" :key="l.value" :value="l.value">{{ l.label }}</option>
      </select>
    </div>

    <Card class="w-full max-w-sm">
      <CardHeader class="text-center">
        <div class="mx-auto mb-2 flex size-11 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Building2 v-if="mode === 'iam'" class="size-6" />
          <ShieldCheck v-else class="size-6" />
        </div>
        <CardTitle class="text-xl">{{ title }}</CardTitle>
        <CardDescription>{{ subtitle }}</CardDescription>
      </CardHeader>
      <CardContent>
        <form class="flex flex-col gap-4" @submit.prevent="onSubmit">
          <div v-if="mode === 'iam'" class="flex flex-col gap-2">
            <Label for="domain">{{ t("auth.domain") }}</Label>
            <Input id="domain" v-model="domain" autocomplete="off" placeholder="acme.example.com" />
          </div>
          <div class="flex flex-col gap-2">
            <Label for="username">{{ t("auth.username") }}</Label>
            <Input id="username" v-model="username" autocomplete="username" />
          </div>
          <div class="flex flex-col gap-2">
            <Label for="password">{{ t("auth.password") }}</Label>
            <Input id="password" v-model="password" type="password" autocomplete="current-password" />
          </div>
          <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
          <Button type="submit" :disabled="loading" class="w-full">
            {{ loading ? t("auth.loggingIn") : t("auth.login") }}
          </Button>
        </form>

        <div class="mt-5 border-t pt-4 text-center">
          <button
            v-if="mode === 'iam'"
            type="button"
            class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
            @click="switchMode('root')"
          >
            <ShieldCheck class="size-4" /> {{ t("auth.switchToRoot") }}
          </button>
          <button
            v-else
            type="button"
            class="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
            @click="switchMode('iam')"
          >
            <Building2 class="size-4" /> {{ t("auth.switchToIam") }}
          </button>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
