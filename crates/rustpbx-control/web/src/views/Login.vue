<script setup lang="ts">
import { ref } from "vue";
import { useRouter, useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { useAuthStore } from "@/stores/auth";
import { SUPPORTED_LOCALES, setLocale, type AppLocale } from "@/i18n";
import { ApiError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";

const { t, locale } = useI18n();
const auth = useAuthStore();
const router = useRouter();
const route = useRoute();

const username = ref("admin");
const password = ref("");
const error = ref("");
const loading = ref(false);

function changeLocale(e: Event) {
  setLocale((e.target as HTMLSelectElement).value as AppLocale);
}

async function onSubmit() {
  error.value = "";
  loading.value = true;
  try {
    await auth.login(username.value, password.value);
    const redirect = (route.query.redirect as string) || "/admin/dashboard";
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
        <div class="mx-auto mb-2 text-3xl">📞</div>
        <CardTitle class="text-xl">{{ t("auth.loginTitle") }}</CardTitle>
        <CardDescription>{{ t("auth.loginSubtitle") }}</CardDescription>
      </CardHeader>
      <CardContent>
        <form class="flex flex-col gap-4" @submit.prevent="onSubmit">
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
      </CardContent>
    </Card>
  </div>
</template>
