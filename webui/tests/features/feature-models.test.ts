import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import type {
  A2dpGuard,
  ConfirmService,
  NotificationCenter,
  StatusCoordinator
} from "../../src/application";
import { createOperationCoordinator } from "../../src/application";
import { deviceStatusFromControllerStatus, type DeviceStatus } from "../../src/domain/device-status";
import { createPolicyModel } from "../../src/features/policy/createPolicyModel";
import { createToolsModel, type ToolsController } from "../../src/features/tools/createToolsModel";
import { createTuningModel } from "../../src/features/tuning/createTuningModel";
import { createSettingsModel, detectLanguage } from "../../src/features/settings/createSettingsModel";

function notifications(events: string[] = []): NotificationCenter {
  const [toasts] = createSignal([]);
  const [log, setLog] = createSignal("");
  return {
    toasts,
    log,
    show: (message, tone) => { events.push(`${tone ?? "info"}:${message}`); return message; },
    success: (message) => { events.push(`success:${message}`); return message; },
    error: (error) => { events.push(`error:${error instanceof Error ? error.message : String(error)}`); return "error"; },
    info: (message) => { events.push(`info:${message}`); return message; },
    dismiss: () => undefined,
    clear: () => undefined,
    setLog,
    dispose: () => undefined
  };
}

function confirmations(accepted = true): ConfirmService {
  const [request] = createSignal(undefined);
  return {
    request,
    confirm: vi.fn().mockResolvedValue(accepted),
    accept: () => undefined,
    cancel: () => undefined,
    dispose: () => undefined
  };
}

function a2dp(routeFailure = false): A2dpGuard {
  return {
    isRouteFailure: ({ code }) => routeFailure || code === 72,
    confirmBeforeMutation: vi.fn().mockResolvedValue(true),
    handleRouteFailure: vi.fn().mockResolvedValue("dismissed")
  };
}

function statusCoordinator(onRefresh?: (scope: unknown) => void): StatusCoordinator<DeviceStatus> {
  const initial = deviceStatusFromControllerStatus({});
  const [status, setStatus] = createSignal<DeviceStatus>(initial);
  return {
    status,
    read: vi.fn(),
    hydrate: vi.fn(),
    refresh: vi.fn(async (scope) => { onRefresh?.(scope); return status(); }),
    tryRefresh: vi.fn().mockResolvedValue(true),
    replaceStatus: setStatus
  };
}

describe("Policy model", () => {
  it("rejects invalid rates without executing the controller", async () => {
    const apply = vi.fn();
    const model = createPolicyModel({
      controller: { apply, reset: vi.fn() },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(),
      a2dp: a2dp(),
      statusCoordinator: statusCoordinator()
    });
    model.update("rate", "custom");
    model.update("customRate", "1");
    await model.apply();
    expect(apply).not.toHaveBeenCalled();
  });

  it("preserves the exit 72 recovery and policy refresh flow", async () => {
    const guard = a2dp();
    const refreshed: unknown[] = [];
    const model = createPolicyModel({
      controller: { apply: vi.fn().mockResolvedValue({ code: 72, stdout: "route", stderr: "" }), reset: vi.fn() },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(),
      a2dp: guard,
      statusCoordinator: statusCoordinator((scope) => refreshed.push(scope))
    });
    await model.apply();
    expect(guard.confirmBeforeMutation).toHaveBeenCalledOnce();
    expect(guard.handleRouteFailure).toHaveBeenCalledWith("policy.section");
    expect(refreshed).toEqual(["policy"]);
  });
});

describe("Tools model", () => {
  it("normalizes resampler and USB drafts and refreshes before success", async () => {
    const events: string[] = [];
    const applyResampler = vi.fn().mockResolvedValue({ code: 0, stdout: "ok", stderr: "" });
    const setUsbPeriod = vi.fn().mockResolvedValue({ code: 0, stdout: "ok", stderr: "" });
    const controller: ToolsController = {
      setBluetoothHal: vi.fn(),
      applyResampler,
      resetResampler: vi.fn(),
      setUsbPeriod,
      resetUsbPeriod: vi.fn(),
      diagnose: vi.fn()
    };
    const model = createToolsModel({
      controller,
      operations: createOperationCoordinator(),
      notifications: notifications(events),
      confirmations: confirmations(),
      a2dp: a2dp(),
      status: { refresh: vi.fn(async () => { events.push("refresh"); return deviceStatusFromControllerStatus({}); }) }
    });

    model.setResamplerPreset("custom");
    model.setResamplerStopBand("999");
    model.setResamplerHalfLength("413");
    model.setResamplerCheat(false);
    model.setResamplerPercent("150");
    await model.applyResampler();
    expect(applyResampler).toHaveBeenCalledWith({ preset: "custom", bypass: "none", cheat: false, stopBand: 242, halfLength: 416, percent: 100 });
    expect(events.indexOf("refresh")).toBeLessThan(events.indexOf("success:tools.resampler.success"));

    model.setUsbPeriod("2310");
    await model.applyUsbPeriod();
    expect(setUsbPeriod).toHaveBeenCalledWith(2250);
    expect(model.usbPeriod()).toBe("2250");
  });

  it("normalizes drafts with controller-provided numeric limits", async () => {
    const applyResampler = vi.fn().mockResolvedValue({ code: 0, stdout: "ok", stderr: "" });
    const setUsbPeriod = vi.fn().mockResolvedValue({ code: 0, stdout: "ok", stderr: "" });
    const model = createToolsModel({
      controller: {
        setBluetoothHal: vi.fn(),
        applyResampler,
        resetResampler: vi.fn(),
        setUsbPeriod,
        resetUsbPeriod: vi.fn(),
        diagnose: vi.fn()
      },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(),
      a2dp: a2dp(),
      status: { refresh: vi.fn().mockResolvedValue({}) },
      limits: {
        usbPeriod: () => ({ min: 250, max: 1000, step: 250 }),
        resampler: () => ({
          stopBand: { min: 40, max: 200, step: 10 },
          halfLength: { min: 16, max: 320, step: 16 },
          cutoffPercent: { min: 10, max: 90, step: 5 },
          cheatPercent: { min: 20, max: 150, step: 5 }
        })
      }
    });

    model.setResamplerPreset("custom");
    model.setResamplerCheat(false);
    model.setResamplerStopBand("77");
    model.setResamplerHalfLength("53");
    model.setResamplerPercent("94");
    await model.applyResampler();
    expect(applyResampler).toHaveBeenCalledWith({
      preset: "custom",
      bypass: "none",
      cheat: false,
      stopBand: 80,
      halfLength: 48,
      percent: 90
    });

    model.setUsbPeriod("610");
    await model.applyUsbPeriod();
    expect(setUsbPeriod).toHaveBeenCalledWith(500);
  });
});

describe("Tuning model", () => {
  it("uses schema reset features and retains unrelated pending drafts", async () => {
    const model = createTuningModel({
      controller: {
        setJitter: vi.fn(),
        resetJitter: vi.fn().mockResolvedValue({ code: 0, stdout: "ok", stderr: "" })
      },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(),
      a2dp: a2dp(),
      statusCoordinator: statusCoordinator(),
      language: () => "en",
      translate: (value) => value,
      resetFeatures: () => new Set(["doze"])
    });
    model.updateFeature("doze", true);
    model.updateFeature("effect", true);
    await model.reset();
    expect(model.dirty()).toEqual(["effect"]);
    expect(model.settings().jitter.effect).toBe(true);
  });

  it("executes sequentially and retains the failed and later dirty features", async () => {
    const setJitter = vi.fn()
      .mockResolvedValueOnce({ code: 0, stdout: "doze ok", stderr: "" })
      .mockResolvedValueOnce({ code: 1, stdout: "vm failed", stderr: "" });
    const refresh = vi.fn();
    const model = createTuningModel({
      controller: { setJitter, resetJitter: vi.fn() },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(),
      a2dp: a2dp(),
      statusCoordinator: statusCoordinator(() => refresh()),
      language: () => "zh-CN",
      translate: (value) => value
    });
    model.updateFeature("doze", true);
    model.updateFeature("vm", true);
    model.updateFeature("wifi", true);
    await model.apply();
    expect(setJitter.mock.calls.map(([operation]) => operation.feature)).toEqual(["doze", "vm"]);
    expect(model.dirty()).toEqual(["vm", "wifi"]);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("does not execute high-risk changes when confirmation is declined", async () => {
    const setJitter = vi.fn();
    const model = createTuningModel({
      controller: { setJitter, resetJitter: vi.fn() },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirmations(false),
      a2dp: a2dp(),
      statusCoordinator: statusCoordinator(),
      language: () => "zh-CN",
      translate: (value) => value
    });
    model.updateFeature("thermal", true);
    await model.apply();
    expect(setJitter).not.toHaveBeenCalled();
    expect(model.dirty()).toEqual(["thermal"]);
  });

  it("uses schema-provided high-risk metadata for new jitter features", async () => {
    const setJitter = vi.fn();
    const confirm = confirmations(false);
    const model = createTuningModel({
      controller: { setJitter, resetJitter: vi.fn() },
      operations: createOperationCoordinator(),
      notifications: notifications(),
      confirmations: confirm,
      a2dp: a2dp(),
      statusCoordinator: statusCoordinator(),
      language: () => "en",
      translate: (value) => value,
      highRiskFeatures: () => new Set(["vendor-danger"]),
      featureLabelKey: (feature) => `jitter.${feature}.label`
    });
    model.updateFeature("vendor-danger", true);
    await model.apply();
    expect(confirm.confirm).toHaveBeenCalledOnce();
    expect(setJitter).not.toHaveBeenCalled();
  });
});

describe("Settings model", () => {
  it("detects language and only commits auto reapply after controller success", async () => {
    expect(detectLanguage(["en-US", "zh-Hans-CN"])).toBe("zh-CN");
    await new Promise<void>((resolve, reject) => createRoot((dispose) => {
      const values = new Map<string, string>();
      const model = createSettingsModel({
        controller: { setAutoReapply: vi.fn().mockResolvedValue({ code: 0, stdout: "", stderr: "" }) },
        operations: createOperationCoordinator(),
        notifications: notifications(),
        confirmations: confirmations(),
        storage: { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); } },
        navigatorLanguages: ["en-US"],
        documentElement: { lang: "" }
      });
      void model.changeAutoReapply(true).then(() => {
        expect(model.autoReapply()).toBe(true);
        model.setPendingLanguage("zh-CN");
        model.applyLanguage();
        expect(values.get("usbSrLanguage")).toBe("zh-CN");
        dispose();
        resolve();
      }).catch(reject);
    }));
  });
});
