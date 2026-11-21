import { useSyncExternalStore } from "react";
import zhCN from "@/locales/zh-CN.json";

export type Language = "en" | "zh-CN";
const storageKey = "opcode-ui-language";
const messages: Record<string, string> = zhCN;

function initialLanguage(): Language {
  try {
    const saved = localStorage.getItem(storageKey);
    if (saved === "en" || saved === "zh-CN") return saved;
  } catch {
    // Storage may be unavailable in a browser preview.
  }
  return typeof navigator !== "undefined" && /^zh(?:-|$)/i.test(navigator.language) ? "zh-CN" : "en";
}

let language = initialLanguage();
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};

export const getLanguage = () => language;

export function setLanguage(next: Language) {
  if (language === next) return;
  language = next;
  document.documentElement.lang = next;
  try {
    localStorage.setItem(storageKey, next);
  } catch {
    // The current session still uses the selected language.
  }
  listeners.forEach(listener => listener());
}

export function useLanguage() {
  return useSyncExternalStore(subscribe, getLanguage);
}

export function t(message: string, values?: Record<string, string | number>): string {
  const translated = language === "zh-CN" && Object.prototype.hasOwnProperty.call(messages, message)
    ? messages[message]
    : message;
  return values ? translated.replace(/\{(\w+)\}/g, (match, name) => String(values[name] ?? match)) : translated;
}
