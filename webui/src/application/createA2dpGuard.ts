type ExecResultLike = {
  code: number;
  stdout: string;
  stderr: string;
};

type StatusResponseLike<TStatus> = {
  result: ExecResultLike;
  status: TStatus;
};

export type A2dpController<TStatus> = {
  status: () => Promise<StatusResponseLike<TStatus>>;
  openBluetoothSettings: () => Promise<ExecResultLike>;
};

export type A2dpConfirm = {
  confirm: (options: {
    title: string;
    message: string;
    confirmLabel?: string;
    showCancel?: boolean;
  }) => Promise<boolean>;
};

export type A2dpNotifications = {
  success: (message: string) => unknown;
  error: (error: unknown) => unknown;
};

export type A2dpRecoveryResult = "settings-opened" | "dismissed";
type A2dpState = "connected" | "disconnected" | "unknown";

export type A2dpGuardOptions<TStatus> = {
  controller: A2dpController<TStatus>;
  confirmations: A2dpConfirm;
  notifications: A2dpNotifications;
  isConnected?: (status: TStatus) => boolean;
  onStatus?: (status: TStatus) => void;
  translate?: Translator;
};

export type A2dpGuard = {
  isRouteFailure: (result: Pick<ExecResultLike, "code">) => boolean;
  confirmBeforeMutation: () => Promise<boolean>;
  handleRouteFailure: (action: string) => Promise<A2dpRecoveryResult>;
};

function defaultA2dpState(status: unknown): A2dpState {
  if (!status || typeof status !== "object") return "unknown";
  const value = status as {
    bluetooth_a2dp_connected?: unknown;
    bluetooth_a2dp_state?: unknown;
    system?: { a2dpConnected?: unknown; a2dpState?: unknown };
  };
  const state = value.bluetooth_a2dp_state ?? value.system?.a2dpState;
  if (state === "connected" || state === "disconnected" || state === "unknown") return state;
  const legacy = value.bluetooth_a2dp_connected ?? value.system?.a2dpConnected;
  if (legacy === "1" || legacy === true) return "connected";
  if (legacy === "0" || legacy === false) return "disconnected";
  return "unknown";
}

function resultError(result: ExecResultLike, fallback: string): Error {
  return new Error(result.stderr || result.stdout || fallback);
}

/** Centralizes preflight A2DP confirmation and controller exit-code 72 recovery. */
export function createA2dpGuard<TStatus>(options: A2dpGuardOptions<TStatus>): A2dpGuard {
  const translate = options.translate ?? ((message: string) => message);

  async function confirmBeforeMutation(): Promise<boolean> {
    const { result, status } = await options.controller.status();
    if (result.code !== 0) throw resultError(result, translate("a2dp.status.readFailed"));
    options.onStatus?.(status);
    const state = options.isConnected
      ? options.isConnected(status) ? "connected" : "disconnected"
      : defaultA2dpState(status);
    if (state === "disconnected") return true;

    if (state === "unknown") {
      return options.confirmations.confirm({
        title: "a2dp.unknown.title",
        message: "a2dp.unknown.message",
        confirmLabel: "a2dp.unknown.continue"
      });
    }

    return options.confirmations.confirm({
      title: "a2dp.connected.title",
      message: "a2dp.connected.message",
      confirmLabel: "a2dp.connected.continue"
    });
  }

  async function handleRouteFailure(action: string): Promise<A2dpRecoveryResult> {
    const openSettings = await options.confirmations.confirm({
      title: "a2dp.failure.title",
      message: translate("a2dp.failure.message", { action: translate(action) }),
      confirmLabel: "a2dp.failure.openSettings"
    });

    if (!openSettings) {
      options.notifications.error("a2dp.failure.manualReconnect");
      return "dismissed";
    }

    const result = await options.controller.openBluetoothSettings();
    if (result.code !== 0) throw resultError(result, translate("a2dp.settings.openFailed"));
    options.notifications.success("a2dp.settings.opened");
    return "settings-opened";
  }

  return {
    isRouteFailure: (result) => result.code === 72,
    confirmBeforeMutation,
    handleRouteFailure
  };
}
import type { Translator } from "../shared/types";
