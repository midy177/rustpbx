import { createApp } from "vue";
import { createPinia } from "pinia";
import { router } from "@/router";
import { i18n, setLocale } from "@/i18n";
import App from "@/App.vue";
import "@/style.css";

// Apply saved/detected locale to <html lang>.
setLocale(i18n.global.locale.value as "zh-CN" | "en");

createApp(App).use(createPinia()).use(router).use(i18n).mount("#app");
