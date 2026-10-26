import type { PolicySettings } from "./models";
import { rateOptions, policyOptionGroups } from "./options";
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

export function policyTemplateValue(policy: string): string {
  return policy.replace(/-dynamic$/, "");
}

export function policyOptionsForMode<T extends { value: string }>(
  options: ReadonlyArray<T>,
  policy: string
): T[] {
  const dynamic = policy.endsWith("-dynamic");
  return options.filter((option) => option.value.endsWith("-dynamic") === dynamic);
}

export function policyWithDynamicCompatibility(
  policy: string,
  enabled: boolean,
  availablePolicies: ReadonlyArray<string>
): string {
  const available = new Set(availablePolicies);
  const dynamic = availablePolicies.filter((value) => value.endsWith("-dynamic"));
  const standard = availablePolicies.filter((value) => !value.endsWith("-dynamic"));
  if (enabled) {
    if (policy.endsWith("-dynamic") && available.has(policy)) return policy;
    const matching = `${policy}-dynamic`;
    if (available.has(matching)) return matching;
    if (available.has("offload-direct-dynamic")) return "offload-direct-dynamic";
    return dynamic[0] ?? policy;
  }
  if (!policy.endsWith("-dynamic") && available.has(policy)) return policy;
  const matching = policyTemplateValue(policy);
  if (available.has(matching)) return matching;
  if (available.has("auto")) return "auto";
  return standard[0] ?? policy;
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

// Keep capability filtering from the controller, but use a stable presentation
// order with a final home for future options unknown to this WebUI.
export function groupPolicyOptions<T extends { value: string }>(options: ReadonlyArray<T>) {
  const known = new Set<string>(policyOptionGroups.flatMap(({ values }) => [...values]));
  return policyOptionGroups.map(({ labelKey, values }) => ({
    labelKey,
    options: [
      ...values.flatMap((value) => options.filter((option) => option.value === value)),
      ...(labelKey === "policy.group.other" ? options.filter(({ value }) => !known.has(value)) : [])
    ]
  })).filter(({ options }) => options.length > 0);
}
