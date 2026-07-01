<script setup lang="ts">
import { computed } from "vue";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    type?: "button" | "submit" | "reset";
    variant?: "default" | "outline" | "ghost";
    size?: "default" | "sm" | "icon";
    disabled?: boolean;
  }>(),
  {
    type: "button",
    variant: "default",
    size: "default",
    disabled: false,
  },
);

const classes = computed(() =>
  cn(
    "inline-flex items-center justify-center gap-2 rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
    props.variant === "default" && "bg-primary text-primary-foreground hover:bg-primary/90",
    props.variant === "outline" && "border border-input bg-background hover:bg-accent hover:text-accent-foreground",
    props.variant === "ghost" && "hover:bg-accent hover:text-accent-foreground",
    props.size === "default" && "h-10 px-4 py-2",
    props.size === "sm" && "h-9 px-3",
    props.size === "icon" && "h-10 w-10",
  ),
);
</script>

<template>
  <button :type="type" :class="classes" :disabled="disabled">
    <slot />
  </button>
</template>
