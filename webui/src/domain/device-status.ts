import type {
  ControllerOperationKind,
  ControllerOperationResult,
  ControllerOperationState,
  ControllerStatus
} from "../platform/controller-protocol";
import type { ControllerSchema } from "../platform/controller-schema";
import type { PolicySettings } from "./models";
import { defaultPolicySettings, policySettingsFromStatus } from "./policy";
import {
  defaultToolsSettings,
  toolsSettingsFromStatus,
  type ToolsSettings
} from "./tools";
import {
  defaultTuningSettings,
  tuningSettingsFromStatus,
  type TuningSettings
} from "./tuning";

export type DeviceSystemStatus = {
  controllerVersion?: string;
  scriptVersion?: string;
  moduleDir?: string;
  audioserverPid?: string;
  a2dpConnected: boolean;
  a2dpState: "connected" | "disconnected" | "unknown";
  namespaceOk: boolean;
  lastAction?: string;
  lastRoute?: string;
  lastExit?: number;
  lastTime?: string;
  stateDegraded: boolean;
  stateDegradedReason?: string;
  stateMigration?: string;
  stateRecoveryReason?: string;
  lastCommandLog?: string;
  lastTimedOut: boolean;
  lastStdoutTruncated: boolean;
  lastStderrTruncated: boolean;
  lastUpstreamStatus?: string;
  lastPostCheck?: string;
  lastStatePersist?: string;
  lastOperationApplied?: boolean;
  operationState?: ControllerOperationState;
  lastOperationKind?: ControllerOperationKind;
  lastOperationResult?: ControllerOperationResult;
  lastOperationState?: ControllerOperationState;
  lastAuditStatus?: string;
};

export type DeviceStatus = {
  policy: PolicySettings;
  tools: ToolsSettings;
  tuning: TuningSettings;
  autoReapply: boolean;
  system: DeviceSystemStatus;
};

export function statusFlag(value: string | string[] | undefined): boolean {
  return value === "1";
}

function optionalFlag(value: string | undefined, previous = false): boolean {
  return value === undefined ? previous : value === "1";
}

function optionalExitCode(value: string | undefined): number | undefined {
  if (value === undefined || value.trim() === "") return undefined;
  const numeric = Number(value);
  return Number.isInteger(numeric) ? numeric : undefined;
}

function operationState(value: string | undefined): DeviceSystemStatus["operationState"] {
  return value === "not_started" || value === "applied" || value === "partially_applied" || value === "possibly_applied"
    ? value
    : undefined;
}

function operationKind(value: string | undefined): ControllerOperationKind | undefined {
  return value === "query" || value === "mutation" ? value : undefined;
}

function operationResult(value: string | undefined): ControllerOperationResult | undefined {
  return value === "success" || value === "failed" || value === "timed_out" || value === "not_started"
    ? value
    : undefined;
}

function a2dpState(
  state: string | undefined,
  legacyConnected: string | undefined,
  previous?: DeviceSystemStatus["a2dpState"]
): DeviceSystemStatus["a2dpState"] {
  if (state === "connected" || state === "disconnected" || state === "unknown") return state;
  if (legacyConnected !== undefined) return statusFlag(legacyConnected) ? "connected" : "disconnected";
  return previous ?? "unknown";
}

function policyDefaults(schema?: ControllerSchema): PolicySettings {
  const fallback = defaultPolicySettings();
  if (!schema) return fallback;
  const rate = schema.sampleRates.find((option) => option.default)?.value
    ?? schema.sampleRates[0]?.value
    ?? fallback.rate;
  const bitDepth = schema.bitDepths.find((option) => option.default)?.value
    ?? schema.bitDepths[0]?.value
    ?? fallback.bitDepth;
  const switchDefault = (value: string) => schema.switches.find((option) => option.value === value)?.default === true;
  return {
    policy: schema.policy.default,
    rate,
    customRate: rate,
    bitDepth,
    drc: switchDefault("drc"),
    forceUsbv2: switchDefault("force_usbv2"),
    forceBluetoothQti: switchDefault("force_bluetooth_qti")
  };
}

function toolsDefaults(schema?: ControllerSchema): ToolsSettings {
  const fallback = defaultToolsSettings();
  if (!schema) return fallback;
  const custom = schema.extras.resamplerCustomDefault;
  return {
    bluetoothHal: schema.extras.bluetoothHalDefault
      ?? schema.extras.bluetoothHal.find((option) => option.default)?.value
      ?? fallback.bluetoothHal,
    resampler: {
      preset: schema.extras.resamplerDefaultPreset
        ?? schema.extras.resamplerPresets.find((option) => option.default)?.value
        ?? fallback.resampler.preset,
      bypass: custom?.bypass ?? fallback.resampler.bypass,
      cheat: custom?.cheat ?? fallback.resampler.cheat,
      stopBand: custom?.stopBand ?? fallback.resampler.stopBand,
      halfLength: custom?.halfLength ?? fallback.resampler.halfLength,
      percent: custom?.percent ?? fallback.resampler.percent
    },
    usbPeriod: schema.extras.usbPeriodDefault ?? fallback.usbPeriod,
    diagnostic: schema.extras.diagnosticDefault
      ?? schema.extras.diagnostics.find((option) => option.default)?.value
      ?? fallback.diagnostic,
    diagnosticAll: schema.extras.diagnosticsCompleteOutputDefault ?? fallback.diagnosticAll
  };
}

function tuningDefaults(schema?: ControllerSchema): TuningSettings {
  const fallback = defaultTuningSettings();
  if (!schema) return fallback;
  const defaults = schema.extras.jitterDefaults;
  return {
    jitter: Object.fromEntries(schema.extras.jitterFeatures.map(({ value, default: defaultValue }) => [
      value,
      defaults?.features[value] ?? defaultValue === true
    ])),
    ioScheduler: defaults?.ioScheduler
      ?? schema.extras.ioSchedulers.find((option) => option.default)?.value
      ?? fallback.ioScheduler,
    ioTone: defaults?.ioTone
      ?? schema.extras.ioTones.find((option) => option.default)?.value
      ?? fallback.ioTone,
    wifiNoRestart: defaults?.wifiNoRestart ?? fallback.wifiNoRestart
  };
}

export function deviceStatusFromControllerStatus(
  status: ControllerStatus,
  previous?: DeviceStatus,
  schema?: ControllerSchema
): DeviceStatus {
  const defaultPolicy = policyDefaults(schema);
  const defaultTools = toolsDefaults(schema);
  const defaultTuning = tuningDefaults(schema);
  const previousPolicy = previous ? { ...defaultPolicy, ...previous.policy } : defaultPolicy;
  const previousTools = previous
    ? { ...defaultTools, ...previous.tools, resampler: { ...defaultTools.resampler, ...previous.tools.resampler } }
    : defaultTools;
  const previousTuning = previous
    ? { ...defaultTuning, ...previous.tuning, jitter: { ...defaultTuning.jitter, ...previous.tuning.jitter } }
    : defaultTuning;
  const standardRates = schema
    ? new Set(schema.sampleRates.map(({ value }) => value))
    : undefined;
  const jitterFeatures = schema?.extras.jitterFeatures.map(({ value }) => value);
  const currentA2dpState = a2dpState(
    status.bluetooth_a2dp_state,
    status.bluetooth_a2dp_connected,
    previous?.system.a2dpState
  );

  return {
    policy: policySettingsFromStatus(status, previousPolicy, standardRates),
    tools: toolsSettingsFromStatus(
      status,
      previousTools,
      schema?.limits,
      schema?.extras.resamplerDefaultPreset
    ),
    tuning: tuningSettingsFromStatus(status, previousTuning, jitterFeatures),
    autoReapply: status.auto_reapply === undefined
      ? previous?.autoReapply ?? false
      : statusFlag(status.auto_reapply),
    system: {
      controllerVersion: status.controller_version ?? previous?.system.controllerVersion,
      scriptVersion: status.script_version ?? previous?.system.scriptVersion,
      moduleDir: status.module_dir ?? previous?.system.moduleDir,
      audioserverPid: status.audioserver_pid ?? previous?.system.audioserverPid,
      a2dpConnected: currentA2dpState === "connected",
      a2dpState: currentA2dpState,
      namespaceOk: status.namespace_ok === undefined
        ? previous?.system.namespaceOk ?? false
        : statusFlag(status.namespace_ok),
      lastAction: status.last_action ?? previous?.system.lastAction,
      lastRoute: status.last_route ?? previous?.system.lastRoute,
      lastExit: status.last_exit === undefined
        ? previous?.system.lastExit
        : optionalExitCode(status.last_exit),
      lastTime: status.last_time ?? previous?.system.lastTime,
      stateDegraded: optionalFlag(status.state_degraded, previous?.system.stateDegraded ?? false),
      stateDegradedReason: status.state_degraded_reason ?? previous?.system.stateDegradedReason,
      stateMigration: status.state_migration ?? previous?.system.stateMigration,
      stateRecoveryReason: status.state_recovery_reason ?? previous?.system.stateRecoveryReason,
      lastCommandLog: status.last_command_log ?? previous?.system.lastCommandLog,
      lastTimedOut: optionalFlag(status.last_timed_out, previous?.system.lastTimedOut ?? false),
      lastStdoutTruncated: optionalFlag(status.last_stdout_truncated, previous?.system.lastStdoutTruncated ?? false),
      lastStderrTruncated: optionalFlag(status.last_stderr_truncated, previous?.system.lastStderrTruncated ?? false),
      lastUpstreamStatus: status.last_upstream_status ?? previous?.system.lastUpstreamStatus,
      lastPostCheck: status.last_post_check ?? previous?.system.lastPostCheck,
      lastStatePersist: status.last_state_persist ?? previous?.system.lastStatePersist,
      lastOperationApplied: status.last_operation_applied === undefined
        ? previous?.system.lastOperationApplied
        : statusFlag(status.last_operation_applied),
      operationState: status.operation_state === undefined
        ? previous?.system.operationState
        : operationState(status.operation_state),
      lastOperationKind: status.last_operation_kind === undefined
        ? previous?.system.lastOperationKind
        : operationKind(status.last_operation_kind),
      lastOperationResult: status.last_operation_result === undefined
        ? previous?.system.lastOperationResult
        : operationResult(status.last_operation_result),
      lastOperationState: status.last_operation_state === undefined
        ? previous?.system.lastOperationState
        : operationState(status.last_operation_state),
      lastAuditStatus: status.last_audit_status ?? previous?.system.lastAuditStatus
    }
  };
}
