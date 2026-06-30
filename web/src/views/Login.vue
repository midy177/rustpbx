<script setup lang="ts">
import { ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAuthStore } from "@/stores/auth";

const auth = useAuthStore();
const route = useRoute();
const router = useRouter();
const username = ref("");
const password = ref("");
const tenant = ref("");
const error = ref("");
const submitting = ref(false);

async function submit() {
  error.value = "";
  submitting.value = true;
  try {
    await auth.login(username.value, password.value, tenant.value || undefined);
    await router.push((route.query.redirect as string | undefined) ?? "/");
  } catch (err) {
    error.value = err instanceof Error ? err.message : "Login failed";
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <main class="grid min-h-screen place-items-center bg-muted/30 px-4">
    <Card class="w-full max-w-md">
      <CardHeader>
        <CardTitle>CloudPBX</CardTitle>
      </CardHeader>
      <CardContent>
        <form class="space-y-4" @submit.prevent="submit">
          <div class="space-y-2">
            <Label for="tenant">Tenant</Label>
            <Input id="tenant" v-model="tenant" autocomplete="organization" placeholder="default" />
          </div>
          <div class="space-y-2">
            <Label for="username">Username</Label>
            <Input id="username" v-model="username" autocomplete="username" />
          </div>
          <div class="space-y-2">
            <Label for="password">Password</Label>
            <Input id="password" v-model="password" type="password" autocomplete="current-password" />
          </div>
          <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
          <Button type="submit" class="w-full" :disabled="submitting">
            {{ submitting ? "Signing in" : "Sign in" }}
          </Button>
        </form>
      </CardContent>
    </Card>
  </main>
</template>
