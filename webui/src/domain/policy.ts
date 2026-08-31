import type { PolicySettings } from "./models";
import { rateOptions } from "./options";
import type { ControllerStatus } from "../platform/controller-protocol";

export const defaultPolicySettings = (): PolicySettings => ({
  policy: "auto",
  rate: "44100",
  customRate: "44100",
  bitDepth: "32",
  drc: false,
  forceUsbv2: false,
  forceBluetoothQti: false
});

export type PolicyValidationResult =
  | { valid: true; sampleRate: number }
  | { valid: false; reason: "sample-rate-out-of-range" };

export function selectedRate(settings: PolicySettings): string {
  return settings.rate === "custom" ? settings.customRate : settings.rate;
}

export function displayRate(value: string): string {
  const known = rateOptions.find(([rate]) => rate === value);
  if (known) return known[1];
  const numeric = Number(value);
  return Number.isFinite(numeric) ? `${numeric.toLocaleString()} Hz` : "status.unset";
}

export function validatePolicySettings(settings: PolicySettings, limit = { min: 44100, max: 768000 }): PolicyValidationResult {
  const sampleRate = Number(selectedRate(settings));
  if (!Number.isInteger(sampleRate) || sampleRate < limit.min || sampleRate > limit.max) {
    return { valid: false, reason: "sample-rate-out-of-range" };
  }
  return { valid: true, sampleRate };
}

export function policySettingsFromStatus(
  status: ControllerStatus,
  previous: PolicySettings = defaultPolicySettings(),
  standardRates: ReadonlySet<string> = new Set(rateOptions.map(([rate]) => rate))
): PolicySettings {
  const settings = { ...previous };
  if (status.policy) settings.policy = status.policy;
  if (status.sample_rate) {
    const known = standardRates.has(status.sample_rate);
    settings.rate = known ? status.sample_rate : "custom";
    settings.customRate = status.sample_rate;
  }
  if (status.bit_depth) settings.bitDepth = status.bit_depth;
  if (status.drc !== undefined) settings.drc = status.drc === "1";
  if (status.force_usbv2 !== undefined) settings.forceUsbv2 = status.force_usbv2 === "1";
  if (status.force_bluetooth_qti !== undefined) {
    settings.forceBluetoothQti = status.force_bluetooth_qti === "1";
  }
  return settings;
}
