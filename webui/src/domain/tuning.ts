import type { ControllerStatus } from "../platform/controller-protocol";
import {
  defaultJitterValues,
  jitterFeatures,
  type JitterFeature,
  type JitterValues
} from "./options";

export type TuningSettings = {
  jitter: JitterValues;
  ioScheduler: string;
  ioTone: string;
  wifiNoRestart: boolean;
};

export type JitterOperation = {
  feature: JitterFeature;
  enabled: boolean;
  ioScheduler?: string;
  ioTone?: string;
  wifiNoRestart?: boolean;
};

export const defaultTuningSettings = (): TuningSettings => ({
  jitter: defaultJitterValues(),
  ioScheduler: "*",
  ioTone: "medium",
  wifiNoRestart: false
});

export function tuningSettingsFromStatus(
  status: ControllerStatus,
  previous: TuningSettings = defaultTuningSettings(),
  features: readonly JitterFeature[] = jitterFeatures.map(([feature]) => feature)
): TuningSettings {
  return {
    jitter: Object.fromEntries(features.map((feature) => [
      feature,
      status[`jitter_${feature}`] === undefined
        ? previous.jitter[feature] ?? false
        : status[`jitter_${feature}`] === "1"
    ])) as JitterValues,
    ioScheduler: status.io_scheduler ?? previous.ioScheduler,
    ioTone: status.io_tone ?? previous.ioTone,
    wifiNoRestart: status.wifi_no_restart === undefined
      ? previous.wifiNoRestart
      : status.wifi_no_restart === "1"
  };
}

export function markJitterDirty(
  dirty: readonly JitterFeature[],
  feature: JitterFeature
): JitterFeature[] {
  return dirty.includes(feature) ? [...dirty] : [...dirty, feature];
}

export function buildJitterOperation(
  feature: JitterFeature,
  settings: TuningSettings
): JitterOperation {
  const enabled = settings.jitter[feature];
  return {
    feature,
    enabled,
    ...(feature === "io" && enabled
      ? { ioScheduler: settings.ioScheduler, ioTone: settings.ioTone }
      : {}),
    ...(feature === "wifi" && enabled && settings.wifiNoRestart
      ? { wifiNoRestart: true }
      : {})
  };
}

export function buildJitterOperations(
  features: readonly JitterFeature[],
  settings: TuningSettings
): JitterOperation[] {
  return features.map((feature) => buildJitterOperation(feature, settings));
}

export function requiresHighRiskConfirmation(
  operationOrOperations: JitterOperation | readonly JitterOperation[],
  highRiskFeatures: ReadonlySet<string> = new Set(["selinux", "thermal"])
): boolean {
  const operations = Array.isArray(operationOrOperations)
    ? operationOrOperations
    : [operationOrOperations];
  return operations.some(({ feature, enabled }) =>
    enabled && highRiskFeatures.has(feature)
  );
}
