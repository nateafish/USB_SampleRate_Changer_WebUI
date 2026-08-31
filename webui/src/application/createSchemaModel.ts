import { createSignal, type Accessor } from "solid-js";
import {
  bitOptions,
  bluetoothHalOptions,
  diagnosticOptions,
  ioSchedulerOptions,
  ioToneOptions,
  jitterFeatures,
  policyOptions,
  rateOptions,
  resamplerBypassOptions,
  resamplerPresetGroups
} from "../domain/options";
import type { ControllerClient } from "../platform/controller-client";
import type {
  ControllerSchema,
  NumericRange,
  SchemaOption,
  SchemaToolName
} from "../platform/controller-schema";

export type SchemaGroup = { labelKey: string; options: ReadonlyArray<SchemaOption> };

export type SchemaModel = {
  schema: Accessor<ControllerSchema | undefined>;
  policyOptions: Accessor<ReadonlyArray<SchemaOption>>;
  rateOptions: Accessor<ReadonlyArray<SchemaOption>>;
  bitDepthOptions: Accessor<ReadonlyArray<SchemaOption>>;
  policySwitchOptions: Accessor<ReadonlyArray<SchemaOption>>;
  bluetoothHalOptions: Accessor<ReadonlyArray<SchemaOption>>;
  resamplerPresetGroups: Accessor<ReadonlyArray<SchemaGroup>>;
  resamplerBypassOptions: Accessor<ReadonlyArray<SchemaOption>>;
  diagnosticOptions: Accessor<ReadonlyArray<SchemaOption>>;
  jitterFeatures: Accessor<ReadonlyArray<SchemaOption>>;
  ioSchedulerOptions: Accessor<ReadonlyArray<SchemaOption>>;
  ioToneOptions: Accessor<ReadonlyArray<SchemaOption>>;
  toolAvailable: (tool: SchemaToolName) => boolean;
  toolOperation: (tool: SchemaToolName, operation: string) => boolean;
  diagnosticsCompleteOutput: Accessor<boolean>;
  jitterWifiNoRestart: Accessor<boolean>;
  jitterFeatureCapability: (feature: string, capability: string) => boolean;
  jitterHighRiskFeatures: Accessor<ReadonlySet<string>>;
  jitterAudioRestartFeatures: Accessor<ReadonlySet<string>>;
  jitterResetFeatures: Accessor<ReadonlySet<string>>;
  jitterFeatureLabelKey: (feature: string) => string;
  sampleRateLimit: Accessor<NumericRange>;
  usbPeriodLimit: Accessor<NumericRange>;
  resamplerLimits: Accessor<ControllerSchema["limits"]["resampler"]>;
  load: () => Promise<boolean>;
};

function fallback(items: ReadonlyArray<readonly [string, ...unknown[]]>, prefix: string, suffix = ""): SchemaOption[] {
  return items.map(([value]) => ({ value, labelKey: `${prefix}.${value}${suffix}` }));
}

const fallbackPolicy = fallback(policyOptions, "policy.option", ".label");
const fallbackRates = fallback(rateOptions, "rate");
const fallbackBits = fallback(bitOptions, "format");
const fallbackSwitches: SchemaOption[] = [
  { value: "drc", labelKey: "switch.drc.label", descriptionKey: "switch.drc.description" },
  { value: "force_usbv2", labelKey: "switch.force_usbv2.label", descriptionKey: "switch.force_usbv2.description" },
  { value: "force_bluetooth_qti", labelKey: "switch.force_bluetooth_qti.label", descriptionKey: "switch.force_bluetooth_qti.description" }
];
const fallbackBluetooth = fallback(bluetoothHalOptions, "bluetooth_hal.option", ".label");
const fallbackBypass = fallback(resamplerBypassOptions, "resampler.bypass", ".label");
const fallbackDiagnostics = fallback(diagnosticOptions, "diagnostic", ".label");
const fallbackJitter = jitterFeatures.map(([value]) => ({
  value,
  labelKey: `jitter.${value}.label`,
  descriptionKey: `jitter.${value}.description`,
  highRisk: value === "selinux" || value === "thermal",
  requiresAudioRestart: value === "effect",
  capabilities: {
    io_parameters: value === "io",
    wifi_no_restart: value === "wifi"
  }
}));
const fallbackSchedulers = fallback(ioSchedulerOptions, "jitter.io_scheduler", ".label");
const fallbackTones = fallback(ioToneOptions, "jitter.io_tone", ".label");
const fallbackPresetGroups: SchemaGroup[] = resamplerPresetGroups.map(({ label, options }) => ({
  labelKey: label,
  options: fallback(options, "resampler.preset", ".label").map((option) => option.value === "custom"
    ? { ...option, labelKey: "resampler.custom.label" }
    : { ...option, descriptionKey: `resampler.preset.${option.value}.description` })
}));

const fallbackLimits = {
  sampleRate: { min: 44100, max: 768000, step: 1 },
  usbPeriod: { min: 125, max: 50000, step: 125 },
  resampler: {
    stopBand: { min: 20, max: 242, step: 1 },
    halfLength: { min: 8, max: 640, step: 8 },
    cutoffPercent: { min: 1, max: 100, step: 1 },
    cheatPercent: { min: 1, max: 200, step: 1 }
  }
};

function withFallback(dynamic: ReadonlyArray<SchemaOption> | undefined, fallbackItems: ReadonlyArray<SchemaOption>) {
  return dynamic?.length ? dynamic : fallbackItems;
}

function groupPresets(items: ReadonlyArray<SchemaOption>): SchemaGroup[] {
  const custom = fallbackPresetGroups[0].options.find(({ value }) => value === "custom")!;
  // The controller's "default" alias resolves to the same AudioFlinger
  // preset as 179-408-99 (Android 12+ default); never expose it as a choice.
  const selectableItems = items.filter(({ value }) => value !== "default");
  const combined = selectableItems.some(({ value }) => value === "custom")
    ? selectableItems
    : [...selectableItems, custom];
  const fallbackGroup = new Map(fallbackPresetGroups.flatMap((group) => group.options.map(({ value }) => [value, group.labelKey] as const)));
  const groups = new Map<string, SchemaOption[]>();
  for (const item of combined) {
    const key = item.group ? (item.group.includes(".") ? item.group : `resampler.group.${item.group}.label`) : fallbackGroup.get(item.value) ?? "resampler.group.standard.label";
    groups.set(key, [...(groups.get(key) ?? []), item]);
  }
  return [...groups].map(([labelKey, options]) => ({ labelKey, options }));
}

const supportedPolicySwitches = new Set(fallbackSwitches.map(({ value }) => value));

function schemaFeatures(current: ControllerSchema | undefined): ReadonlyArray<SchemaOption> {
  return withFallback(current?.extras.jitterFeatures, fallbackJitter);
}

export function createSchemaModel(controller: Pick<ControllerClient, "schema">): SchemaModel {
  const [schema, setSchema] = createSignal<ControllerSchema>();

  async function load(): Promise<boolean> {
    try {
      const response = await controller.schema();
      if (response.schema) {
        setSchema(response.schema);
        return true;
      }
    } catch {
      // Older controllers and malformed contracts use the complete static fallback.
    }
    return false;
  }

  function toolAvailable(name: SchemaToolName): boolean {
    return schema()?.extras.tools[name]?.available ?? true;
  }

  function toolOperation(name: SchemaToolName, operation: string): boolean {
    const capability = schema()?.extras.tools[name];
    if (capability?.available === false) return false;
    return capability?.operations[operation] ?? true;
  }

  function jitterFeatureCapability(feature: string, capability: string): boolean {
    const option = schemaFeatures(schema()).find(({ value }) => value === feature);
    return option?.capabilities?.[capability] ?? false;
  }

  return {
    schema,
    policyOptions: () => schema()?.policy.options ?? fallbackPolicy,
    rateOptions: () => schema()?.sampleRates ?? fallbackRates,
    bitDepthOptions: () => schema()?.bitDepths ?? fallbackBits,
    policySwitchOptions: () => {
      const dynamic = schema()?.switches.filter(({ value }) => supportedPolicySwitches.has(value));
      return withFallback(dynamic, fallbackSwitches);
    },
    bluetoothHalOptions: () => withFallback(schema()?.extras.bluetoothHal, fallbackBluetooth),
    resamplerPresetGroups: () => groupPresets(withFallback(schema()?.extras.resamplerPresets, fallbackPresetGroups.flatMap(({ options }) => options))),
    resamplerBypassOptions: () => withFallback(schema()?.extras.resamplerBypass, fallbackBypass),
    diagnosticOptions: () => withFallback(schema()?.extras.diagnostics, fallbackDiagnostics),
    jitterFeatures: () => schemaFeatures(schema()),
    ioSchedulerOptions: () => withFallback(schema()?.extras.ioSchedulers, fallbackSchedulers),
    ioToneOptions: () => withFallback(schema()?.extras.ioTones, fallbackTones),
    toolAvailable,
    toolOperation,
    diagnosticsCompleteOutput: () => schema()?.extras.diagnosticsCompleteOutput ?? true,
    jitterWifiNoRestart: () => schema()?.extras.jitterWifiNoRestart ?? true,
    jitterFeatureCapability,
    jitterHighRiskFeatures: () => new Set(schemaFeatures(schema()).filter(({ highRisk }) => highRisk).map(({ value }) => value)),
    jitterAudioRestartFeatures: () => new Set(schemaFeatures(schema()).filter(({ requiresAudioRestart }) => requiresAudioRestart).map(({ value }) => value)),
    jitterResetFeatures: () => new Set(schema()?.extras.jitterResetFeatures ?? fallbackJitter.slice(0, 9).map(({ value }) => value)),
    jitterFeatureLabelKey: (feature) => schemaFeatures(schema()).find(({ value }) => value === feature)?.labelKey ?? `jitter.${feature}.label`,
    sampleRateLimit: () => schema()?.limits.sampleRate ?? fallbackLimits.sampleRate,
    usbPeriodLimit: () => schema()?.limits.usbPeriod ?? fallbackLimits.usbPeriod,
    resamplerLimits: () => schema()?.limits.resampler ?? fallbackLimits.resampler,
    load
  };
}
