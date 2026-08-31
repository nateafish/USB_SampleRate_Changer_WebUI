import { english } from "./locales/en";
import { chinese } from "./locales/zh-CN";
import type { TranslationParams } from "./shared/types";

export type Language = "zh-CN" | "en";

function interpolate(value: string, params?: TranslationParams): string {
  if (!params) return value;
  return value.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, key: string) => String(params[key] ?? match));
}

/**
 * English is the canonical source language. Chinese overrides are applied only
 * after a stable semantic key resolves, so missing translations fall back to
 * English instead of leaking Chinese source text into the English interface.
 */
export function translate(language: Language, key: string, params?: TranslationParams): string {
  const englishSource = english[key as keyof typeof english] ?? key;
  const localized = language === "zh-CN"
    ? chinese[key as keyof typeof chinese] ?? englishSource
    : englishSource;
  return interpolate(localized, params);
}

export function translateRuntime(language: Language, value: string, params?: TranslationParams): string {
  return translate(language, value, params);
}
