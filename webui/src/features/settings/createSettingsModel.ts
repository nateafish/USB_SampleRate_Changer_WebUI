import { createEffect, createSignal, type Accessor } from "solid-js";
import type { Language } from "../../i18n";
import { translate } from "../../i18n";
import type { ConfirmService, NotificationCenter, OperationCoordinator } from "../../application";
import type { DeviceStatus } from "../../domain/device-status";
import {
  controllerExecutionError,
  type OperationAwareExecResult
} from "../../platform/controller-errors";
import type { Translator } from "../../shared/types";

export type LocalizedOption = readonly [string, string];

export type SettingsController = {
  setAutoReapply(enabled: boolean): Promise<OperationAwareExecResult>;
};

export type SettingsPageModel = {
  language: Accessor<Language>;
  pendingLanguage: Accessor<Language>;
  autoReapply: Accessor<boolean>;
  tx: Translator;
  localizeOptions: (options: ReadonlyArray<LocalizedOption>) => ReadonlyArray<LocalizedOption>;
  setPendingLanguage: (value: Language) => void;
  applyLanguage: () => void;
  changeAutoReapply: (value: boolean) => Promise<void>;
  hydrate: (status: DeviceStatus) => void;
};

export type SettingsModelOptions = {
  controller: SettingsController;
  operations: OperationCoordinator;
  notifications: NotificationCenter;
  confirmations: ConfirmService;
  storage?: Pick<Storage, "getItem" | "setItem">;
  navigatorLanguages?: readonly string[];
  documentElement?: Pick<HTMLElement, "lang">;
};

export function detectLanguage(locales: readonly string[]): Language {
  return locales.some((locale) => /^zh(?:-|$)/i.test(locale)) ? "zh-CN" : "en";
}

export function createSettingsModel(options: SettingsModelOptions): SettingsPageModel {
  const storage = options.storage ?? window.localStorage;
  const documentElement = options.documentElement ?? document.documentElement;
  const storedLanguage = storage.getItem("usbSrLanguage");
  const detected = detectLanguage(options.navigatorLanguages ?? [
    ...(Array.isArray(navigator.languages) ? navigator.languages : []),
    navigator.language
  ].filter((locale): locale is string => typeof locale === "string" && locale.length > 0));
  const initialLanguage: Language = storedLanguage === "en" || storedLanguage === "zh-CN"
    ? storedLanguage
    : detected;
  const [language, setLanguage] = createSignal<Language>(initialLanguage);
  const [pendingLanguage, setPendingLanguage] = createSignal<Language>(initialLanguage);
  const [autoReapply, setAutoReapply] = createSignal(false);

  createEffect(() => {
    documentElement.lang = language();
  });

  const tx: Translator = (value, params) => translate(language(), value, params);

  function localizeOptions(optionItems: ReadonlyArray<LocalizedOption>): ReadonlyArray<LocalizedOption> {
    return optionItems.map(([value, label]) => [value, tx(label)] as const);
  }

  function applyLanguage() {
    const nextLanguage = pendingLanguage();
    setLanguage(nextLanguage);
    storage.setItem("usbSrLanguage", nextLanguage);
  }

  async function changeAutoReapply(value: boolean): Promise<void> {
    if (value) {
      const accepted = await options.confirmations.confirm({
        title: "settings.reapply.confirmTitle",
        message: "settings.reapply.confirmMessage",
        confirmLabel: "settings.reapply.confirm"
      });
      if (!accepted) return;
    }

    await options.operations.runExclusive("settings.auto-reapply", async () => {
      try {
        const result = await options.controller.setAutoReapply(value);
        if (result.code !== 0) {
          throw controllerExecutionError(result, "settings.reapply.saveFailed");
        }
        setAutoReapply(value);
        options.notifications.success(value ? "settings.reapply.enabled" : "settings.reapply.disabled");
      } catch (error) {
        options.notifications.error(error);
      }
    });
  }

  function hydrate(status: DeviceStatus) {
    setAutoReapply(status.autoReapply);
  }

  return {
    language,
    pendingLanguage,
    autoReapply,
    tx,
    localizeOptions,
    setPendingLanguage,
    applyLanguage,
    changeAutoReapply,
    hydrate
  };
}
