import type { ControllerStatus } from "../platform/controller-protocol";

export type BluetoothHal = string;
export type ResamplerPreset = string;
export type ResamplerBypass = string;
export type DiagnosticType = string;

export type ResamplerSettings = {
  preset: ResamplerPreset;
  bypass: ResamplerBypass;
  cheat: boolean;
  stopBand: number;
  halfLength: number;
  percent: number;
};

export type ToolsSettings = {
  bluetoothHal: BluetoothHal;
  resampler: ResamplerSettings;
  usbPeriod: number;
  diagnostic: DiagnosticType;
  diagnosticAll: boolean;
};

type NumericInput = number | string | null | undefined;
export type NumericRange = { min: number; max: number; step: number };
export type ToolNumericLimits = {
  usbPeriod: NumericRange;
  resampler: {
    stopBand: NumericRange;
    halfLength: NumericRange;
    cutoffPercent: NumericRange;
    cheatPercent: NumericRange;
  };
};

function normalizeSteppedValue(
  value: NumericInput,
  minimum: number,
  maximum: number,
  step: number,
  fallback: number
): number {
  const numeric = Number(value);
  // Schema-provided ranges may not contain the historical default. Normalize
  // that fallback through the same clamp/step path before returning it.
  const fallbackValue = Number.isFinite(fallback) ? fallback : minimum;
  if (!Number.isFinite(numeric)) {
    const fallbackStepped = minimum + Math.round((fallbackValue - minimum) / step) * step;
    return Math.min(maximum, Math.max(minimum, fallbackStepped));
  }
  const stepped = minimum + Math.round((numeric - minimum) / step) * step;
  return Math.min(maximum, Math.max(minimum, stepped));
}

export const defaultResamplerSettings = (): ResamplerSettings => ({
  preset: "179-408-99",
  bypass: "none",
  cheat: true,
  stopBand: 179,
  halfLength: 408,
  percent: 99
});

export const defaultToolsSettings = (): ToolsSettings => ({
  bluetoothHal: "offload",
  resampler: defaultResamplerSettings(),
  usbPeriod: 2250,
  diagnostic: "audio",
  diagnosticAll: false
});

export function normalizeUsbPeriod(value: NumericInput, range: NumericRange = { min: 125, max: 50000, step: 125 }): number {
  return normalizeSteppedValue(value, range.min, range.max, range.step, 2250);
}

export function normalizeResamplerStopBand(value: NumericInput, range: NumericRange = { min: 20, max: 242, step: 1 }): number {
  return normalizeSteppedValue(value, range.min, range.max, range.step, 179);
}

export function normalizeResamplerHalfLength(value: NumericInput, range: NumericRange = { min: 8, max: 640, step: 8 }): number {
  return normalizeSteppedValue(value, range.min, range.max, range.step, 408);
}

export function normalizeResamplerPercent(value: NumericInput, cheat: boolean, range?: NumericRange): number {
  const selected = range ?? { min: 1, max: cheat ? 200 : 100, step: 1 };
  return normalizeSteppedValue(value, selected.min, selected.max, selected.step, 99);
}

export function toolsSettingsFromStatus(
  status: ControllerStatus,
  previous: ToolsSettings = defaultToolsSettings(),
  limits?: ToolNumericLimits,
  defaultResamplerPreset?: string
): ToolsSettings {
  const cheat = status.resampler_cheat === undefined
    ? previous.resampler.cheat
    : status.resampler_cheat === "1";

  return {
    bluetoothHal: status.bluetooth_hal
      ? status.bluetooth_hal
      : previous.bluetoothHal,
    resampler: {
      preset: status.resampler_preset
        ? status.resampler_preset === "default" && defaultResamplerPreset
          ? defaultResamplerPreset
          : status.resampler_preset
        : previous.resampler.preset,
      bypass: status.resampler_bypass
        ? status.resampler_bypass
        : previous.resampler.bypass,
      cheat,
      stopBand: status.resampler_stop_band === undefined
        ? previous.resampler.stopBand
        : normalizeResamplerStopBand(status.resampler_stop_band, limits?.resampler.stopBand),
      halfLength: status.resampler_half_length === undefined
        ? previous.resampler.halfLength
        : normalizeResamplerHalfLength(status.resampler_half_length, limits?.resampler.halfLength),
      percent: status.resampler_percent === undefined
        ? previous.resampler.percent
        : normalizeResamplerPercent(status.resampler_percent, cheat, cheat ? limits?.resampler.cheatPercent : limits?.resampler.cutoffPercent)
    },
    usbPeriod: status.usb_period === undefined
      ? previous.usbPeriod
      : normalizeUsbPeriod(status.usb_period, limits?.usbPeriod),
    diagnostic: status.diagnostic
      ? status.diagnostic
      : previous.diagnostic,
    diagnosticAll: status.diagnostic_all === undefined
      ? previous.diagnosticAll
      : status.diagnostic_all === "1"
  };
}
