import { describe, expect, it, vi } from "vitest";
import type { ControllerSchema } from "../../src/platform/controller-schema";
import { createSchemaModel } from "../../src/application/createSchemaModel";

const dynamicSchema: ControllerSchema = {
  schemaVersion: 1,
  apiVersion: 1,
  capabilities: { json_schema: true },
  limits: {
    sampleRate: { min: 48000, max: 192000, step: 2, integer: true },
    usbPeriod: { min: 250, max: 4000, step: 250 },
    resampler: {
      stopBand: { min: 40, max: 200, step: 2 },
      halfLength: { min: 16, max: 320, step: 16 },
      cutoffPercent: { min: 10, max: 90, step: 5 },
      cheatPercent: { min: 20, max: 150, step: 5 }
    }
  },
  policy: { default: "dynamic", options: [{ value: "dynamic", labelKey: "policy.dynamic" }] },
  sampleRates: [{ value: "48000", labelKey: "rate.48000" }],
  bitDepths: [{ value: "24", labelKey: "format.24" }],
  switches: [
    { value: "drc", labelKey: "switch.drc.label" },
    { value: "test", labelKey: "switch.test.label" }
  ],
  extras: {
    bluetoothHal: [{ value: "dynamic-hal", labelKey: "hal.dynamic" }],
    resamplerPresets: [
      { value: "179-408-99", labelKey: "resampler.preset.179-408-99.label", group: "standard" },
      { value: "custom", labelKey: "resampler.custom.label", group: "standard" }
    ],
    resamplerBypass: [{ value: "dynamic-bypass", labelKey: "bypass.dynamic" }],
    diagnostics: [{ value: "dynamic-diagnostic", labelKey: "diagnostic.dynamic" }],
    jitterFeatures: [
      { value: "danger", labelKey: "jitter.danger.label", highRisk: true },
      { value: "restart", labelKey: "jitter.restart.label", requiresAudioRestart: true },
      { value: "io", labelKey: "jitter.io.label", capabilities: { io_parameters: true } }
    ],
    ioSchedulers: [{ value: "dynamic", labelKey: "scheduler.dynamic" }],
    ioTones: [{ value: "dynamic", labelKey: "tone.dynamic" }],
    tools: {
      bluetoothHal: { available: false, operations: { set: false } },
      resampler: { available: true, operations: { set_preset: true, set_custom: false, reset: false } },
      diagnostics: { available: true, operations: { run: true } }
    },
    diagnosticsCompleteOutput: false,
    jitterWifiNoRestart: false,
    jitterResetFeatures: ["danger", "io"]
  }
};

describe("schema model", () => {
  it("retains Direct PCM dynamic in fallback and controller-driven menus", async () => {
    const model = createSchemaModel({ schema: vi.fn().mockResolvedValue({
      result: { code: 0, stdout: "{}", stderr: "" },
      schema: { ...dynamicSchema, policy: { default: "auto", options: [
        { value: "offload-direct", labelKey: "policy.option.offload-direct.label" },
        { value: "offload-direct-dynamic", labelKey: "policy.option.offload-direct-dynamic.label" }
      ] } }
    }) });
    expect(model.policyOptions().map(({ value }) => value)).toContain("offload-direct-dynamic");
    await model.load();
    expect(model.policyOptions().map(({ value }) => value)).toEqual(["offload-direct", "offload-direct-dynamic"]);
  });
  it("keeps complete static fallbacks until a valid contract is loaded", async () => {
    const model = createSchemaModel({
      schema: vi.fn().mockResolvedValue({ result: { code: 0, stdout: "", stderr: "" } })
    });

    expect(model.policyOptions().length).toBeGreaterThan(1);
    expect(model.toolAvailable("bluetoothHal")).toBe(true);
    expect(model.toolOperation("resampler", "reset")).toBe(true);
    expect(model.diagnosticsCompleteOutput()).toBe(true);
    expect([...model.jitterResetFeatures()]).toContain("battery");
    expect([...model.jitterResetFeatures()]).toContain("effect");
    await model.load();
    expect(model.policyOptions().length).toBeGreaterThan(1);
  });

  it("applies dynamic options, ranges and capability gates without exposing untyped switches", async () => {
    const model = createSchemaModel({
      schema: vi.fn().mockResolvedValue({
        result: { code: 0, stdout: "{}", stderr: "" },
        schema: dynamicSchema
      })
    });

    await model.load();
    expect(model.policyOptions().map(({ value }) => value)).toEqual(["dynamic"]);
    expect(model.policySwitchOptions().map(({ value }) => value)).toEqual(["drc"]);
    expect(model.usbPeriodLimit()).toEqual({ min: 250, max: 4000, step: 250 });
    expect(model.toolAvailable("bluetoothHal")).toBe(false);
    expect(model.toolOperation("bluetoothHal", "set")).toBe(false);
    expect(model.toolOperation("resampler", "set_preset")).toBe(true);
    expect(model.toolOperation("resampler", "set_custom")).toBe(false);
    expect(model.resamplerPresetGroups().flatMap(({ options }) => options.map(({ value }) => value))).toEqual(["179-408-99", "custom"]);
    expect(model.diagnosticsCompleteOutput()).toBe(false);
    expect(model.jitterWifiNoRestart()).toBe(false);
    expect(model.jitterFeatureCapability("io", "io_parameters")).toBe(true);
    expect([...model.jitterHighRiskFeatures()]).toEqual(["danger"]);
    expect([...model.jitterAudioRestartFeatures()]).toEqual(["restart"]);
    expect([...model.jitterResetFeatures()]).toEqual(["danger", "io"]);
    expect(model.jitterFeatureLabelKey("danger")).toBe("jitter.danger.label");
  });
});
