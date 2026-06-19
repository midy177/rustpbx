<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { Card, CardContent } from "@/components/ui/card";
import { Construction } from "lucide-vue-next";
const { t } = useI18n();
defineProps<{ titleKey: string }>();
</script>

<template>
  <div class="space-y-4">
    <h2 class="text-2xl font-bold tracking-tight">{{ t(titleKey) }}</h2>
    <Card>
      <CardContent class="flex flex-col items-center gap-3 py-16 text-center">
        <Construction class="size-10 text-muted-foreground" />
        <p class="text-lg font-medium">{{ t("common.comingSoon") }}</p>
        <p class="max-w-md text-sm text-muted-foreground">{{ t("common.comingSoonDesc") }}</p>
      </CardContent>
    </Card>
  </div>
</template>
