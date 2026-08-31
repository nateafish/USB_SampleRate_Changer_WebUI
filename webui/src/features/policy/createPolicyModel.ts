import {
  createSignal,
  getOwner,
  onCleanup,
  type Accessor
} from "solid-js";
import type {
  A2dpGuard,
  ConfirmService,
  NotificationCenter,
  OperationCoordinator,
  StatusCoordinator
} from "../../application";
import type { PolicySettings } from "../../domain/models";
import type { DeviceStatus } from "../../domain/device-status";
import { defaultPolicySettings, validatePolicySettings } from "../../domain/policy";
import { defaultToolsSettings } from "../../domain/tools";
import { defaultTuningSettings } from "../../domain/tuning";
import type { ControllerClient } from "../../platform/controller-client";
import { LocalizedError } from "../../shared/types";
import type { NumericRange } from "../../platform/controller-schema";
import { outputText } from "../../platform/controller-protocol";
import { controllerExecutionError } from "../../platform/controller-errors";

type PolicyController = Pick<ControllerClient, "apply" | "reset">;
type PolicyStatusCoordinator = StatusCoordinator<DeviceStatus>;
type ScheduledTask = ReturnType<typeof setTimeout>;

export type PolicyPageModel = {
  settings: Accessor<PolicySettings>;
  status: Accessor<DeviceStatus>;
  busy: Accessor<boolean>;
  activeOperation: Accessor<string | undefined>;
  update: <K extends keyof PolicySettings>(key: K, value: PolicySettings[K]) => void;
  changeRate: (value: string) => void;
  hydrate: (status: DeviceStatus) => void;
  apply: () => Promise<void>;
  reset: () => Promise<void>;
};

export type CreatePolicyModelOptions = {
  controller: PolicyController;
  operations: OperationCoordinator;
  notifications: NotificationCenter;
  confirmations: ConfirmService;
  a2dp: A2dpGuard;
  statusCoordinator: PolicyStatusCoordinator;
  initialSettings?: PolicySettings;
  customRateWarningDelayMs?: number;
  schedule?: (callback: () => void, delayMs: number) => ScheduledTask;
  cancelSchedule?: (task: ScheduledTask) => void;
  sampleRateLimit?: Accessor<NumericRange>;
};

const EMPTY_STATUS: DeviceStatus = {
  policy: defaultPolicySettings(),
  tools: defaultToolsSettings(),
  tuning: defaultTuningSettings(),
  autoReapply: false,
  system: { a2dpConnected: false, a2dpState: "unknown", namespaceOk: false, stateDegraded: false, lastTimedOut: false, lastStdoutTruncated: false, lastStderrTruncated: false }
};

function commandError(
  result: Awaited<ReturnType<PolicyController["apply"]>>,
  fallbackKey: string
): Error {
  return controllerExecutionError(result, fallbackKey);
}

/**
 * Owns the editable policy draft and every policy mutation workflow.
 *
 * Status refreshes intentionally run inside the root operation slot: the
 * StatusCoordinator does not acquire a lock of its own, so a successful
 * command is not announced until the device state has also been hydrated.
 */
export function createPolicyModel(options: CreatePolicyModelOptions): PolicyPageModel {
  const [settings, setSettings] = createSignal<PolicySettings>(
    options.initialSettings ?? defaultPolicySettings()
  );
  const schedule = options.schedule ?? ((callback, delayMs) => setTimeout(callback, delayMs));
  const cancelSchedule = options.cancelSchedule ?? ((task) => clearTimeout(task));
  const warningDelayMs = options.customRateWarningDelayMs ?? 220;
  let warningTask: ScheduledTask | undefined;

  function update<K extends keyof PolicySettings>(key: K, value: PolicySettings[K]) {
    setSettings((current) => ({ ...current, [key]: value }));
  }

  function changeRate(value: string) {
    update("rate", value);
    if (warningTask !== undefined) {
      cancelSchedule(warningTask);
      warningTask = undefined;
    }
    if (value !== "custom") return;

    warningTask = schedule(() => {
      warningTask = undefined;
      void options.confirmations.confirm({
        title: "policy.customRate.title",
        message: "policy.customRate.message",
        confirmLabel: "policy.customRate.accept",
        showCancel: false
      });
    }, warningDelayMs);
  }

  function hydrate(status: DeviceStatus) {
    setSettings(status.policy);
  }

  async function apply() {
    const current = settings();
    const validation = validatePolicySettings(current, options.sampleRateLimit?.());
    if (!validation.valid) {
      options.notifications.error(new LocalizedError("policy.customRate.invalid"));
      return;
    }

    await options.operations.runExclusive("policy.apply", async () => {
      try {
        if (!await options.a2dp.confirmBeforeMutation()) return;

        options.notifications.info("policy.apply.progress");
        const result = await options.controller.apply(current);
        const text = outputText(result);
        options.notifications.setLog(text || `exit=${result.code}`);

        if (options.a2dp.isRouteFailure(result)) {
          await options.a2dp.handleRouteFailure("policy.section");
          await options.statusCoordinator.refresh("policy");
          return;
        }
        if (result.code !== 0) throw commandError(result, "errors.applyFailed");

        await options.statusCoordinator.refresh("policy");
        options.notifications.success(
          result.stdout.includes("bluetooth_a2dp_before=1")
            ? "policy.apply.a2dpSuccess"
            : "policy.apply.success"
        );
      } catch (error) {
        options.notifications.error(error);
      }
    });
  }

  async function reset() {
    const accepted = await options.confirmations.confirm({
      title: "policy.reset.title",
      message: "policy.reset.message",
      confirmLabel: "common.confirmReset"
    });
    if (!accepted) return;

    await options.operations.runExclusive("policy.reset", async () => {
      options.notifications.info("policy.reset.progress");
      try {
        const result = await options.controller.reset();
        const text = outputText(result);
        options.notifications.setLog(text || `exit=${result.code}`);

        if (options.a2dp.isRouteFailure(result)) {
          await options.a2dp.handleRouteFailure("common.reset");
          await options.statusCoordinator.refresh("policy");
          return;
        }
        if (result.code !== 0) throw commandError(result, "errors.resetFailed");

        await options.statusCoordinator.refresh("policy");
        options.notifications.success("policy.reset.success");
      } catch (error) {
        options.notifications.error(error);
      }
    });
  }

  function dispose() {
    if (warningTask === undefined) return;
    cancelSchedule(warningTask);
    warningTask = undefined;
  }

  if (getOwner()) onCleanup(dispose);

  return {
    settings,
    status: () => options.statusCoordinator.status() ?? EMPTY_STATUS,
    busy: options.operations.busy,
    activeOperation: options.operations.activeOperation,
    update,
    changeRate,
    hydrate,
    apply,
    reset
  };
}
