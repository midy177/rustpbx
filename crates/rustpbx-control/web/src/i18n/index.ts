import { createI18n } from "vue-i18n";
import en from "./locales/en";
import zh from "./locales/zh";

const LOCALE_KEY = "rustpbx.control.locale";

export type AppLocale = "zh-CN" | "en";

export const SUPPORTED_LOCALES: { value: AppLocale; label: string }[] = [
  { value: "zh-CN", label: "中文" },
  { value: "en", label: "English" },
];

function detectLocale(): AppLocale {
  const saved = localStorage.getItem(LOCALE_KEY) as AppLocale | null;
  if (saved === "zh-CN" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

export const i18n = createI18n({
  legacy: false,
  locale: detectLocale(),
  fallbackLocale: "en",
  messages: {
    "zh-CN": zh,
    en,
  },
});

export function setLocale(locale: AppLocale) {
  i18n.global.locale.value = locale;
  localStorage.setItem(LOCALE_KEY, locale);
  document.documentElement.lang = locale;
}
