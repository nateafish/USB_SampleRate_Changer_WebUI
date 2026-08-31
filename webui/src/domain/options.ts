export const policyOptions = [
  ["auto", "policy.option.auto.label", "policy.option.auto.description"],
  ["offload", "policy.option.offload.label", "policy.option.offload.description"],
  ["offload-hifi-playback", "policy.option.offload-hifi-playback.label", "policy.option.offload-hifi-playback.description"],
  ["offload-direct", "policy.option.offload-direct.label", "policy.option.offload-direct.description"],
  ["offload-safer", "policy.option.offload-safer.label", "policy.option.offload-safer.description"],
  ["bypass", "policy.option.bypass.label", "policy.option.bypass.description"],
  ["bypass-safer", "policy.option.bypass-safer.label", "policy.option.bypass-safer.description"],
  ["legacy", "policy.option.legacy.label", "policy.option.legacy.description"],
  ["safe", "policy.option.safe.label", "policy.option.safe.description"],
  ["safest", "policy.option.safest.label", "policy.option.safest.description"],
  ["safest-auto", "policy.option.safest-auto.label", "policy.option.safest-auto.description"],
  ["usb", "policy.option.usb.label", "policy.option.usb.description"]
] as const;

export const policyDetails: Record<string, string> = Object.fromEntries(
  policyOptions.map(([value]) => [value, `policy.option.${value}.detail`])
);

export const rateOptions = [
  ["44100", "44.1 kHz"],
  ["48000", "48 kHz"],
  ["88200", "88.2 kHz"],
  ["96000", "96 kHz"],
  ["176400", "176.4 kHz"],
  ["192000", "192 kHz"],
  ["352800", "352.8 kHz"],
  ["384000", "384 kHz"],
  ["705600", "705.6 kHz"],
  ["768000", "768 kHz"]
] as const;

export const bitOptions = [
  ["16", "16-bit PCM"],
  ["24", "24-bit packed PCM"],
  ["32", "32-bit PCM"],
  ["float", "32-bit float PCM"]
] as const;

export const bluetoothHalOptions = ["offload", "aosp", "legacy", "sysbta"].map((value) => [value, `bluetooth_hal.option.${value}.label`] as const);

const resamplerOptions = [
  ...["159-480-92", "165-360-104", "179-408-99", "194-520-100", "ultra-hifi", "cheap-44", "cheap-44-low", "cheap-48", "cheap-48-low", "cheap-96", "mock-dac-a", "mock-dac-b", "mock-dac-c", "mock-mastering"].map((value) => [value, `resampler.preset.${value}.label`] as const)
] as const;

export const diagnosticOptions = ["audio", "bluetooth", "config", "alsa"].map((value) => [value, `diagnostic.${value}.label`] as const);

export const languageOptions = [["zh-CN", "language.zhCN"], ["en", "language.en"]] as const;
const resamplerCustomOption = ["custom", "resampler.custom.label"] as const;
export const resamplerPresetGroups = [
  { label: "resampler.group.standard.label", options: [...resamplerOptions.slice(0, 5), resamplerCustomOption] },
  { label: "resampler.group.nonlinear.label", options: resamplerOptions.slice(5, 10) },
  { label: "resampler.group.simulated.label", options: resamplerOptions.slice(10) }
] as const;
export const resamplerBypassOptions = ["none", "48", "96"].map((value) => [value, `resampler.bypass.${value}.label`] as const);
export const ioSchedulerOptions = ["*", "none", "noop", "deadline", "mq-deadline", "cfq", "bfq", "kyber"].map((value) => [value, `jitter.io_scheduler.${value}.label`] as const);
export const ioToneOptions = ["light", "m-light", "medium", "boost", "exp"].map((value) => [value, `jitter.io_tone.${value}.label`] as const);

export const jitterFeatures = ["selinux", "thermal", "doze", "governor", "camera", "logd", "io", "vm", "wifi", "battery", "effect"].map((value) => [value, `jitter.${value}.label`, `jitter.${value}.description`] as const);

export type JitterFeature = string;
export type JitterValues = Record<JitterFeature, boolean>;
export type ToolAction = "bluetooth-hal" | "resampler" | "resampler-reset" | "usb-period" | "usb-period-reset" | "jitter" | "jitter-reset";

export const defaultJitterValues = (): JitterValues => Object.fromEntries(
  jitterFeatures.map(([key]) => [key, false])
) as JitterValues;
