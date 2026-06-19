<script setup lang="ts">
import { watch } from "vue";
import { X } from "lucide-vue-next";

const props = defineProps<{ open: boolean; title?: string; description?: string }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

function close() {
  emit("update:open", false);
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}

watch(
  () => props.open,
  (open) => {
    if (open) document.addEventListener("keydown", onKey);
    else document.removeEventListener("keydown", onKey);
  },
);
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog">
      <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center p-4">
        <!-- overlay -->
        <div class="absolute inset-0 bg-black/50 backdrop-blur-sm" @click="close" />
        <!-- content -->
        <div
          role="dialog"
          class="relative z-10 grid w-full max-w-lg gap-4 rounded-lg border bg-background p-6 shadow-lg"
        >
          <div v-if="title || description" class="flex flex-col gap-1.5 text-left">
            <h2 v-if="title" class="text-lg font-semibold leading-none tracking-tight">{{ title }}</h2>
            <p v-if="description" class="text-sm text-muted-foreground">{{ description }}</p>
          </div>

          <div><slot /></div>

          <div v-if="$slots.footer" class="flex justify-end gap-2">
            <slot name="footer" />
          </div>

          <button
            class="absolute right-4 top-4 rounded-sm opacity-70 transition-opacity hover:opacity-100"
            @click="close"
            aria-label="Close"
          >
            <X class="size-4" />
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.dialog-enter-active,
.dialog-leave-active {
  transition: opacity 0.15s ease;
}
.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}
</style>
