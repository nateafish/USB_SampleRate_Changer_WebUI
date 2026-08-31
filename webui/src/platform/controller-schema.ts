import type { ExecResult } from "../domain/models";

export type SchemaOption = {
  value: string;
  labelKey: string;
  descriptionKey?: string;
  group?: string;
  selectable?: boolean;
  recommended?: boolean;
  default?: boolean;
  highRisk?: boolean;
  requiresAudioRestart?: boolean;
  capabilities?: Readonly<Record<string, boolean>>;
};

export type NumericRange = { min: number; max: number; step: number };

export type SchemaToolName = "bluetoothHal" | "resampler" | "usbPeriod" | "diagnostics" | "jitter";
export type SchemaToolCapability = {
  available: boolean;
  operations: Readonly<Record<string, boolean>>;
};

export type ControllerSchema = {
  schemaVersion: number;
  apiVersion: number;
  controllerVersion?: string;
  capabilities: Readonly<Record<string, boolean>>;
  limits: {
    sampleRate: NumericRange & { integer: boolean };
    usbPeriod: NumericRange;
    resampler: {
      stopBand: NumericRange;
      halfLength: NumericRange;
      cutoffPercent: NumericRange;
      cheatPercent: NumericRange;
    };
  };
  policy: { default: string; options: SchemaOption[] };
  sampleRates: SchemaOption[];
  bitDepths: SchemaOption[];
  switches: SchemaOption[];
  extras: {
    bluetoothHal: SchemaOption[];
    resamplerPresets: SchemaOption[];
    resamplerBypass: SchemaOption[];
    diagnostics: SchemaOption[];
    jitterFeatures: SchemaOption[];
    ioSchedulers: SchemaOption[];
    ioTones: SchemaOption[];
    tools: Partial<Record<SchemaToolName, SchemaToolCapability>>;
    diagnosticsCompleteOutput?: boolean;
    diagnosticsCompleteOutputDefault?: boolean;
    bluetoothHalDefault?: string;
    resamplerDefaultPreset?: string;
    resamplerCustomDefault?: {
      bypass: string;
      cheat: boolean;
      stopBand: number;
      halfLength: number;
      percent: number;
    };
    usbPeriodDefault?: number;
    diagnosticDefault?: string;
    jitterDefaults?: {
      features: Readonly<Record<string, boolean>>;
      ioScheduler: string;
      ioTone: string;
      wifiNoRestart: boolean;
    };
    jitterResetFeatures?: ReadonlyArray<string>;
    jitterWifiNoRestart?: boolean;
  };
};

export type SchemaResponse = { result: ExecResult; schema?: ControllerSchema };

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function finite(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function booleans(value: unknown): Record<string, boolean> {
  const source = record(value);
  if (!source) return {};
  return Object.fromEntries(
    Object.entries(source).filter((entry): entry is [string, boolean] => typeof entry[1] === "boolean")
  );
}

function range(value: unknown, defaultStep = 1): NumericRange | undefined {
  const item = record(value);
  const min = finite(item?.min);
  const max = finite(item?.max);
  const step = finite(item?.step) ?? defaultStep;
  return min === undefined || max === undefined || min > max || step <= 0
    ? undefined
    : { min, max, step };
}

function options(value: unknown, fallbackPrefix: string): SchemaOption[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const result: SchemaOption[] = [];
  for (const item of value) {
    if (typeof item === "string") {
      result.push({ value: item, labelKey: `${fallbackPrefix}.${item}` });
      continue;
    }
    const row = record(item);
    const rawValue = row?.value ?? row?.key;
    const optionValue = text(rawValue) ?? (typeof rawValue === "number" ? String(rawValue) : undefined);
    if (!optionValue) return undefined;
    const optionCapabilities = booleans(row?.capabilities);
    result.push({
      value: optionValue,
      labelKey: text(row?.label_key) ?? text(row?.labelKey) ?? `${fallbackPrefix}.${optionValue}`,
      descriptionKey: text(row?.description_key) ?? text(row?.descriptionKey),
      group: text(row?.group),
      selectable: typeof row?.selectable === "boolean" ? row.selectable : undefined,
      recommended: row?.recommended === true,
      default: row?.default === true,
      highRisk: typeof row?.high_risk === "boolean" ? row.high_risk : undefined,
      requiresAudioRestart: typeof row?.requires_audio_restart === "boolean" ? row.requires_audio_restart : undefined,
      capabilities: Object.keys(optionCapabilities).length ? optionCapabilities : undefined
    });
  }
  return result.length ? result : undefined;
}

function tool(value: unknown): SchemaToolCapability | undefined {
  const item = record(value);
  if (!item) return undefined;
  return {
    available: item.available !== false,
    operations: booleans(item.operations)
  };
}

export function parseControllerSchema(output: string): ControllerSchema | undefined {
  let raw: unknown;
  try { raw = JSON.parse(output); } catch { return undefined; }
  const root = record(raw);
  if (finite(root?.schema_version) !== 1 || finite(root?.api_version) !== 1) return undefined;

  const limits = record(root?.limits);
  const sampleRate = range(limits?.sample_rate);
  const usbPeriod = range(limits?.usb_period);
  const resamplerLimits = record(limits?.resampler);
  const stopBand = range(resamplerLimits?.stop_band);
  const halfLength = range(resamplerLimits?.half_length);
  const cutoffPercent = range(resamplerLimits?.cutoff_percent);
  const cheatPercent = range(resamplerLimits?.cheat_percent);
  const policy = record(root?.policy);
  const policyOptions = options(policy?.options, "policy.option");
  const sampleRates = options(root?.sample_rates, "rate");
  const bitDepths = options(root?.bit_depths, "format");
  const switches = options(root?.switches, "switch") ?? [];
  const extras = record(root?.extras);
  const bluetooth = record(extras?.bluetooth_hal);
  const resampler = record(extras?.resampler);
  const diagnostics = record(extras?.diagnostics);
  const jitter = record(extras?.jitter);
  const customResampler = record(resampler?.custom);
  const customResamplerDefault = record(customResampler?.default);
  const jitterDefaults = record(jitter?.defaults);
  const completeOutput = record(diagnostics?.complete_output);
  const wifiNoRestart = record(jitter?.wifi_no_restart);
  const presetGroups = new Map<string, string>();
  if (Array.isArray(resampler?.groups)) {
    for (const rawGroup of resampler.groups) {
      const group = record(rawGroup);
      const labelKey = text(group?.label_key);
      if (!labelKey || !Array.isArray(group?.options)) continue;
      for (const value of group.options) if (typeof value === "string") presetGroups.set(value, labelKey);
    }
  }

  if (!sampleRate || !usbPeriod || !stopBand || !halfLength || !cutoffPercent || !cheatPercent
      || !policy || !policyOptions || !sampleRates || !bitDepths) return undefined;

  const parsedPresets = options(resampler?.presets, "resampler.preset") ?? [];
  const customPreset = options(customResampler ? [customResampler] : undefined, "resampler.preset")?.[0];
  const resamplerPresets = customPreset && !parsedPresets.some(({ value }) => value === customPreset.value)
    ? [...parsedPresets, customPreset]
    : parsedPresets;
  const jitterFeatures = options(jitter?.features, "jitter") ?? [];
  const customStopBand = finite(customResamplerDefault?.stop_band);
  const customHalfLength = finite(customResamplerDefault?.half_length);
  const customPercent = finite(customResamplerDefault?.percent);
  const customBypass = text(customResamplerDefault?.bypass);
  const customMode = text(customResamplerDefault?.mode);
  const resamplerCustomDefault = customStopBand !== undefined
      && customHalfLength !== undefined
      && customPercent !== undefined
      && customBypass
      && (customMode === "cheat" || customMode === "cutoff")
    ? {
        bypass: customBypass,
        cheat: customMode === "cheat",
        stopBand: customStopBand,
        halfLength: customHalfLength,
        percent: customPercent
      }
    : undefined;
  const resetFeatures = Array.isArray(jitter?.reset_features)
    ? jitter.reset_features.filter((value): value is string => typeof value === "string")
    : [];

  return {
    schemaVersion: 1,
    apiVersion: 1,
    controllerVersion: text(root?.controller_version),
    capabilities: booleans(root?.capabilities),
    limits: {
      sampleRate: { ...sampleRate, integer: record(limits?.sample_rate)?.integer !== false },
      usbPeriod,
      resampler: { stopBand, halfLength, cutoffPercent, cheatPercent }
    },
    policy: { default: text(policy.default) ?? policyOptions[0].value, options: policyOptions },
    sampleRates,
    bitDepths,
    switches,
    extras: {
      bluetoothHal: (options(bluetooth?.options ?? bluetooth?.actions, "bluetooth_hal.action") ?? []).filter(({ value }) => value !== "status"),
      resamplerPresets: resamplerPresets.map((option) => ({ ...option, group: presetGroups.get(option.value) ?? option.group })),
      resamplerBypass: options(customResampler?.bypass_options ?? customResampler?.bypass, "resampler.bypass") ?? [],
      diagnostics: options(diagnostics?.types, "diagnostic") ?? [],
      jitterFeatures,
      ioSchedulers: options(jitter?.io_scheduler_options ?? jitter?.io_schedulers, "io.scheduler") ?? [],
      ioTones: options(jitter?.io_tone_options ?? jitter?.io_tones, "io.tone") ?? [],
      tools: {
        bluetoothHal: tool(bluetooth),
        resampler: tool(resampler),
        usbPeriod: tool(record(extras?.usb_period)),
        diagnostics: tool(diagnostics),
        jitter: tool(jitter)
      },
      diagnosticsCompleteOutput: typeof completeOutput?.supported === "boolean"
        ? completeOutput.supported as boolean
        : undefined,
      diagnosticsCompleteOutputDefault: typeof completeOutput?.default === "boolean"
        ? completeOutput.default
        : undefined,
      bluetoothHalDefault: text(bluetooth?.default),
      resamplerDefaultPreset: text(resampler?.default_preset),
      resamplerCustomDefault,
      usbPeriodDefault: finite(record(extras?.usb_period)?.default),
      diagnosticDefault: text(diagnostics?.default),
      jitterDefaults: jitterDefaults
        ? {
            features: Object.fromEntries(jitterFeatures.map(({ value, default: defaultValue }) => [value, defaultValue === true])),
            ioScheduler: text(jitterDefaults.io_scheduler) ?? "*",
            ioTone: text(jitterDefaults.io_tone) ?? "medium",
            wifiNoRestart: jitterDefaults.wifi_no_restart === true
          }
        : undefined,
      jitterResetFeatures: resetFeatures,
      jitterWifiNoRestart: typeof wifiNoRestart?.supported === "boolean"
        ? wifiNoRestart.supported as boolean
        : undefined
    }
  };
}
