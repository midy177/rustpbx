<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { RouterView, useRouter } from "vue-router";
import { useAuthStore } from "@/stores/auth";

const router = useRouter();
const auth = useAuthStore();

// Any authenticated API call that comes back 401 (expired/invalid session)
// dispatches this event — reset session state and bounce to login, preserving
// where the user was so they return after re-auth.
function onUnauthorized() {
  const redirect = router.currentRoute.value.fullPath;
  auth.clearSession();
  if (router.currentRoute.value.name !== "login") {
    router.push({ name: "login", query: redirect ? { redirect } : {} });
  }
}

onMounted(() => window.addEventListener("rustpbx:unauthorized", onUnauthorized));
onUnmounted(() => window.removeEventListener("rustpbx:unauthorized", onUnauthorized));
</script>

<template>
  <RouterView />
</template>
