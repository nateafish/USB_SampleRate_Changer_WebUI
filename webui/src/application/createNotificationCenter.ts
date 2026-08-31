import { createSignal, getOwner, onCleanup, type Accessor } from "solid-js";
import { LocalizedError, type ToastItem, type TranslationParams } from "../shared/types";

export type NotificationTone = "success" | "error" | "info";

export type NotificationCenterOptions = {
  translate?: (message: string, params?: TranslationParams) => string;
  durationMs?: number;
  createId?: () => string;
  schedule?: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  cancelSchedule?: (timer: ReturnType<typeof setTimeout>) => void;
};

export type NotificationCenter = {
  toasts: Accessor<ToastItem[]>;
  log: Accessor<string>;
  show: (message: string, tone?: NotificationTone) => string;
  success: (message: string) => string;
  error: (error: unknown) => string;
  info: (message: string) => string;
  dismiss: (id: string) => void;
  clear: () => void;
  setLog: (value: string) => void;
  dispose: () => void;
};

let notificationSequence = 0;

function errorMessage(error: unknown): { message: string; params?: TranslationParams } {
  if (error instanceof LocalizedError) return { message: error.key, params: error.params };
  return { message: error instanceof Error ? error.message : String(error) };
}

/** Creates an explicitly toned toast queue plus the shared execution log. */
export function createNotificationCenter(
  options: NotificationCenterOptions = {}
): NotificationCenter {
  const [toasts, setToasts] = createSignal<ToastItem[]>([]);
  const [log, setLogSignal] = createSignal("");
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const translate = options.translate ?? ((message: string) => message);
  const durationMs = options.durationMs ?? 4200;
  const createId = options.createId ?? (() => `toast-${Date.now().toString(36)}-${(++notificationSequence).toString(36)}`);
  const schedule = options.schedule ?? ((callback, delay) => setTimeout(callback, delay));
  const cancelSchedule = options.cancelSchedule ?? ((timer) => clearTimeout(timer));
  let disposed = false;

  function dismiss(id: string) {
    const timer = timers.get(id);
    if (timer !== undefined) cancelSchedule(timer);
    timers.delete(id);
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }

  function show(message: string, tone: NotificationTone = "info", params?: TranslationParams): string {
    if (disposed) return "";
    const id = createId();
    const toastTone: ToastItem["tone"] = tone === "info" ? undefined : tone;
    setToasts((current) => [...current, { id, message: translate(message, params), tone: toastTone }]);
    timers.set(id, schedule(() => dismiss(id), durationMs));
    return id;
  }

  function clear() {
    for (const timer of timers.values()) cancelSchedule(timer);
    timers.clear();
    setToasts([]);
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    clear();
  }

  if (getOwner()) onCleanup(dispose);

  return {
    toasts,
    log,
    show,
    success: (message) => show(message, "success"),
    error: (error) => { const localized = errorMessage(error); return show(localized.message, "error", localized.params); },
    info: (message) => show(message, "info"),
    dismiss,
    clear,
    setLog: setLogSignal,
    dispose
  };
}
