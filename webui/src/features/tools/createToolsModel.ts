import { createSignal, type Accessor } from "solid-js";
import type {
  A2dpGuard,
  ConfirmService,
  NotificationCenter,
  OperationCoordinator,
  StatusCoordinator
} from "../../application";
import type { DeviceStatus } from "../../domain/device-status";
import type { ToolAction } from "../../domain/options";
import {
  defaultToolsSettings,
  normalizeResamplerHalfLength,
  normalizeResamplerPercent,
  normalizeResamplerStopBand,
  normalizeUsbPeriod,
  type BluetoothHal,
  type DiagnosticType,
  type ResamplerBypass,
  type ResamplerPreset,
  type ResamplerSettings,
  type ToolNumericLimits,
  type ToolsSettings
} from "../../domain/tools";
import { diagnosticText, outputText } from "../../platform/controller-protocol";
import {
  controllerExecutionError,
  type OperationAwareExecResult
} from "../../platform/controller-errors";

export type ToolsController = {
  setBluetoothHal: (hal: BluetoothHal) => Promise<OperationAwareExecResult>;
  applyResampler: (settings: ResamplerSettings) => Promise<OperationAwareExecResult>;
  resetResampler: () => Promise<OperationAwareExecResult>;
  setUsbPeriod: (period: number) => Promise<OperationAwareExecResult>;
  resetUsbPeriod: () => Promise<OperationAwareExecResult>;
  diagnose: (type: DiagnosticType, complete: boolean) => Promise<OperationAwareExecResult>;
};

export type ToolsStatusCoordinator = Pick<
  StatusCoordinator<DeviceStatus>,
  "refresh"
>;

export type ToolsPageModel = {
  busy: Accessor<boolean>;
  toolAction: Accessor<ToolAction | undefined>;

  bluetoothHal: Accessor<BluetoothHal>;
  setBluetoothHal: (value: string) => void;
  applyBluetoothHal: () => Promise<void>;

  resamplerPreset: Accessor<ResamplerPreset>;
  resamplerBypass: Accessor<ResamplerBypass>;
  resamplerCheat: Accessor<boolean>;
  resamplerStopBand: Accessor<string>;
  resamplerHalfLength: Accessor<string>;
  resamplerPercent: Accessor<string>;
  setResamplerPreset: (value: string) => void;
  setResamplerBypass: (value: string) => void;
  setResamplerCheat: (value: boolean) => void;
  setResamplerStopBand: (value: string) => void;
  normalizeResamplerStopBand: () => void;
  setResamplerHalfLength: (value: string) => void;
  normalizeResamplerHalfLength: () => void;
  setResamplerPercent: (value: string) => void;
  normalizeResamplerPercent: () => void;
  applyResampler: () => Promise<void>;
  resetResampler: () => Promise<void>;

  usbPeriod: Accessor<string>;
  normalizedUsbPeriod: Accessor<string>;
  setUsbPeriod: (value: string) => void;
  normalizeUsbPeriod: () => void;
  stepUsbPeriod: (direction: -1 | 1) => void;
  applyUsbPeriod: () => Promise<void>;
  resetUsbPeriod: () => Promise<void>;

  diagnostic: Accessor<DiagnosticType>;
  diagnosticAll: Accessor<boolean>;
  diagnosticOutput: Accessor<string>;
  setDiagnostic: (value: string) => void;
  setDiagnosticAll: (value: boolean) => void;
  runDiagnostic: () => Promise<void>;

  settings: Accessor<ToolsSettings>;
  hydrate: (status: DeviceStatus) => void;
};

export type ToolsModelOptions = {
  controller: ToolsController;
  operations: OperationCoordinator;
  notifications: NotificationCenter;
  confirmations: ConfirmService;
  a2dp: A2dpGuard;
  status: ToolsStatusCoordinator;
  translate?: (value: string) => string;
  limits?: {
    usbPeriod: () => ToolNumericLimits["usbPeriod"];
    resampler: () => ToolNumericLimits["resampler"];
  };
};

function executionError(result: OperationAwareExecResult, fallback: string): Error {
  return controllerExecutionError(result, fallback);
}

/**
 * Owns the Tools page drafts and controller workflows. Pointer handling for
 * the USB range input intentionally stays in the view because it is a DOM
 * interaction rather than device state or application policy.
 */
export function createToolsModel(options: ToolsModelOptions): ToolsPageModel {
  const defaults = defaultToolsSettings();
  const tx = options.translate ?? ((value: string) => value);
  const [toolAction, setToolAction] = createSignal<ToolAction>();
  const [bluetoothHal, setBluetoothHalSignal] = createSignal<BluetoothHal>(defaults.bluetoothHal);
  const [resamplerPreset, setResamplerPresetSignal] = createSignal<ResamplerPreset>(defaults.resampler.preset);
  const [resamplerBypass, setResamplerBypassSignal] = createSignal<ResamplerBypass>(defaults.resampler.bypass);
  const [resamplerCheat, setResamplerCheatSignal] = createSignal(defaults.resampler.cheat);
  const [resamplerStopBand, setResamplerStopBand] = createSignal(String(defaults.resampler.stopBand));
  const [resamplerHalfLength, setResamplerHalfLength] = createSignal(String(defaults.resampler.halfLength));
  const [resamplerPercent, setResamplerPercent] = createSignal(String(defaults.resampler.percent));
  const [usbPeriod, setUsbPeriod] = createSignal(String(defaults.usbPeriod));
  const [diagnostic, setDiagnosticSignal] = createSignal<DiagnosticType>(defaults.diagnostic);
  const [diagnosticAll, setDiagnosticAll] = createSignal(defaults.diagnosticAll);
  const [diagnosticOutput, setDiagnosticOutput] = createSignal("tools.diagnostics.empty");

  const usbLimit = () => options.limits?.usbPeriod();
  const resamplerLimits = () => options.limits?.resampler();
  const normalizedUsbPeriod = () => String(normalizeUsbPeriod(usbPeriod(), usbLimit()));

  function settings(): ToolsSettings {
    const cheat = resamplerCheat();
    return {
      bluetoothHal: bluetoothHal(),
      resampler: {
        preset: resamplerPreset(),
        bypass: resamplerBypass(),
        cheat,
        stopBand: normalizeResamplerStopBand(resamplerStopBand(), resamplerLimits()?.stopBand),
        halfLength: normalizeResamplerHalfLength(resamplerHalfLength(), resamplerLimits()?.halfLength),
        percent: normalizeResamplerPercent(resamplerPercent(), cheat, cheat ? resamplerLimits()?.cheatPercent : resamplerLimits()?.cutoffPercent)
      },
      usbPeriod: normalizeUsbPeriod(usbPeriod(), usbLimit()),
      diagnostic: diagnostic(),
      diagnosticAll: diagnosticAll()
    };
  }

  function hydrate(status: DeviceStatus) {
    const next = status.tools;
    setBluetoothHalSignal(next.bluetoothHal);
    setResamplerPresetSignal(next.resampler.preset);
    setResamplerBypassSignal(next.resampler.bypass);
    setResamplerCheatSignal(next.resampler.cheat);
    setResamplerStopBand(String(next.resampler.stopBand));
    setResamplerHalfLength(String(next.resampler.halfLength));
    setResamplerPercent(String(next.resampler.percent));
    setUsbPeriod(String(next.usbPeriod));
    setDiagnosticSignal(next.diagnostic);
    setDiagnosticAll(next.diagnosticAll);
  }

  function setBluetoothHal(value: string) {
    if (value) setBluetoothHalSignal(value);
  }

  function setResamplerPreset(value: string) {
    if (value) setResamplerPresetSignal(value);
  }

  function setResamplerBypass(value: string) {
    if (value) setResamplerBypassSignal(value);
  }

  function setResamplerCheat(value: boolean) {
    setResamplerCheatSignal(value);
    setResamplerPercent(String(normalizeResamplerPercent(resamplerPercent(), value, value ? resamplerLimits()?.cheatPercent : resamplerLimits()?.cutoffPercent)));
  }

  function normalizeStopBandDraft() {
    setResamplerStopBand(String(normalizeResamplerStopBand(resamplerStopBand(), resamplerLimits()?.stopBand)));
  }

  function normalizeHalfLengthDraft() {
    setResamplerHalfLength(String(normalizeResamplerHalfLength(resamplerHalfLength(), resamplerLimits()?.halfLength)));
  }

  function normalizePercentDraft() {
    setResamplerPercent(String(normalizeResamplerPercent(resamplerPercent(), resamplerCheat(), resamplerCheat() ? resamplerLimits()?.cheatPercent : resamplerLimits()?.cutoffPercent)));
  }

  function normalizeUsbPeriodDraft() {
    setUsbPeriod(normalizedUsbPeriod());
  }

  function stepUsbPeriod(direction: -1 | 1) {
    const limit = usbLimit();
    const step = limit?.step ?? 125;
    setUsbPeriod(String(normalizeUsbPeriod(normalizeUsbPeriod(usbPeriod(), limit) + direction * step, limit)));
  }

  function setDiagnostic(value: string) {
    if (value) setDiagnosticSignal(value);
  }

  async function runMutation(
    operation: string,
    action: ToolAction,
    execute: () => Promise<OperationAwareExecResult>,
    successMessage: string
  ): Promise<void> {
    await options.operations.runExclusive(operation, async () => {
      setToolAction(action);
      try {
        options.notifications.info("tools.apply.progress");
        const result = await execute();
        const text = outputText(result);
        options.notifications.setLog(text || `exit=${result.code}`);

        if (options.a2dp.isRouteFailure(result)) {
          await options.a2dp.handleRouteFailure("settings.title");
          return;
        }
        if (result.code !== 0) throw executionError(result, "errors.operationFailed");

        // Do not announce completion until the affected controls reflect the
        // controller's applied state.
        await options.status.refresh("tools");
        options.notifications.success(successMessage);
      } catch (error) {
        options.notifications.error(error);
      } finally {
        setToolAction(undefined);
      }
    });
  }

  async function applyBluetoothHal() {
    await runMutation(
      "tools.bluetooth-hal",
      "bluetooth-hal",
      () => options.controller.setBluetoothHal(bluetoothHal()),
      "tools.bluetoothHal.success"
    );
  }

  async function applyResampler() {
    const current = settings().resampler;
    setResamplerStopBand(String(current.stopBand));
    setResamplerHalfLength(String(current.halfLength));
    setResamplerPercent(String(current.percent));
    await runMutation(
      "tools.resampler",
      "resampler",
      () => options.controller.applyResampler(current),
      "tools.resampler.success"
    );
  }

  async function resetResampler() {
    const accepted = await options.confirmations.confirm({
      title: "tools.resampler.reset.title",
      message: "tools.resampler.reset.message",
      confirmLabel: "common.confirmReset"
    });
    if (!accepted) return;
    await runMutation(
      "tools.resampler-reset",
      "resampler-reset",
      () => options.controller.resetResampler(),
      "tools.resampler.reset.success"
    );
  }

  async function applyUsbPeriod() {
    const period = normalizeUsbPeriod(usbPeriod(), usbLimit());
    setUsbPeriod(String(period));
    await runMutation(
      "tools.usb-period",
      "usb-period",
      () => options.controller.setUsbPeriod(period),
      "tools.usbPeriod.success"
    );
  }

  async function resetUsbPeriod() {
    const accepted = await options.confirmations.confirm({
      title: "tools.usbPeriod.reset.title",
      message: "tools.usbPeriod.reset.message",
      confirmLabel: "common.confirmReset"
    });
    if (!accepted) return;
    await runMutation(
      "tools.usb-period-reset",
      "usb-period-reset",
      () => options.controller.resetUsbPeriod(),
      "tools.usbPeriod.reset.success"
    );
  }

  async function runDiagnostic() {
    await options.operations.runExclusive("tools.diagnostic", async () => {
      const pending = tx("tools.diagnostics.progress");
      setDiagnosticOutput(pending);
      try {
        const result = await options.controller.diagnose(diagnostic(), diagnosticAll());
        const text = diagnosticText(result);
        setDiagnosticOutput(text || `exit=${result.code}`);
        if (result.code !== 0) throw executionError(result, "errors.diagnosticFailed");
      } catch (error) {
        if (diagnosticOutput() === pending) {
          setDiagnosticOutput(error instanceof Error ? error.message : String(error));
        }
        options.notifications.error(error);
      }
    });
  }

  return {
    busy: options.operations.busy,
    toolAction,
    bluetoothHal,
    setBluetoothHal,
    applyBluetoothHal,
    resamplerPreset,
    resamplerBypass,
    resamplerCheat,
    resamplerStopBand,
    resamplerHalfLength,
    resamplerPercent,
    setResamplerPreset,
    setResamplerBypass,
    setResamplerCheat,
    setResamplerStopBand,
    normalizeResamplerStopBand: normalizeStopBandDraft,
    setResamplerHalfLength,
    normalizeResamplerHalfLength: normalizeHalfLengthDraft,
    setResamplerPercent,
    normalizeResamplerPercent: normalizePercentDraft,
    applyResampler,
    resetResampler,
    usbPeriod,
    normalizedUsbPeriod,
    setUsbPeriod,
    normalizeUsbPeriod: normalizeUsbPeriodDraft,
    stepUsbPeriod,
    applyUsbPeriod,
    resetUsbPeriod,
    diagnostic,
    diagnosticAll,
    diagnosticOutput,
    setDiagnostic,
    setDiagnosticAll,
    runDiagnostic,
    settings,
    hydrate
  };
}
