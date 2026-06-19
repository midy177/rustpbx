<script setup lang="ts">
defineProps<{ colspan: number }>();
</script>
<template>
  <tr>
    <td :colspan="colspan" class="p-8 text-center text-sm text-muted-foreground">
      <slot />
    </td>
  </tr>
</template>
