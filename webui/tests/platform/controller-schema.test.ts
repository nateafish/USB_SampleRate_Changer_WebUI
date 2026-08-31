import { describe, expect, it } from "vitest";
import { parseControllerSchema } from "../../src/platform/controller-schema";

const valid = JSON.stringify({
  schema_version: 1,
  api_version: 1,
  controller_version: "1.1.0",
  capabilities: { json_schema: true, audio_restart_interface: true, batch_reapply: true },
  limits: {
    sample_rate: { min: 44100, max: 768000, integer: true },
    usb_period: { min: 125, max: 50000, step: 125 },
    resampler: {
      stop_band: { min: 20, max: 242, step: 1 },
      half_length: { min: 8, max: 640, step: 8 },
      cutoff_percent: { min: 1, max: 100, step: 1 },
      cheat_percent: { min: 1, max: 200, step: 1 }
    }
  },
  policy: { default: "auto", options: [{ value: "auto", label_key: "policy.option.auto.label" }] },
  sample_rates: [{ value: 44100, label_key: "rate.44100" }],
  bit_depths: [{ value: "32", label_key: "format.32" }],
  switches: [
    { key: "drc", label_key: "switch.drc.label", description_key: "switch.drc.description" },
    { key: "test", label_key: "switch.test.label" }
  ],
  extras: {
    bluetooth_hal: {
      available: false,
      default: "offload",
      operations: { status: true, set: false, reset: false },
      options: [{ value: "offload", label_key: "bluetooth_hal.option.offload.label", recommended: true }]
    },
    usb_period: {
      available: true,
      default: 2250,
      operations: { status: true, set: true, reset: false }
    },
    resampler: {
      available: true,
      default_preset: "179-408-99",
      operations: { status: true, set_preset: true, set_custom: false, reset: true },
      presets: [{ value: "179-408-99", label_key: "resampler.preset.179-408-99.label" }],
      groups: [{ label_key: "resampler.group.standard.label", options: ["179-408-99"] }],
      custom: {
        value: "custom",
        label_key: "resampler.custom.label",
        default: { bypass: "none", mode: "cheat", stop_band: 179, half_length: 408, percent: 99 },
        bypass_options: [{ value: "none", label_key: "resampler.bypass.none.label" }]
      }
    },
    diagnostics: {
      available: true,
      default: "audio",
      operations: { run: true, status: false, reset: false },
      complete_output: { supported: false, default: false },
      types: [{ value: "audio", label_key: "diagnostic.audio.label" }]
    },
    jitter: {
      available: true,
      operations: { status: true, set: true, reset: true },
      features: [
        { value: "selinux", label_key: "jitter.selinux.label", high_risk: true, requires_audio_restart: false },
        { value: "io", label_key: "jitter.io.label", capabilities: { io_parameters: true, wifi_no_restart: false } },
        { value: "effect", label_key: "jitter.effect.label", requires_audio_restart: true }
      ],
      io_scheduler_options: [{ value: "*", label_key: "jitter.io_scheduler.*.label", default: true }],
      io_tone_options: [{ value: "medium", label_key: "jitter.io_tone.medium.label", default: true }],
      defaults: { io_scheduler: "*", io_tone: "medium", wifi_no_restart: false },
      reset_features: ["selinux", "io"],
      wifi_no_restart: { supported: false, default: false }
    }
  }
});

describe("controller schema", () => {
  it("normalizes the versioned JSON contract", () => {
    const schema = parseControllerSchema(valid);
    expect(schema?.policy.options[0].value).toBe("auto");
    expect(schema?.capabilities.batch_reapply).toBe(true);
    expect(schema?.limits.sampleRate.max).toBe(768000);
    expect(schema?.switches[0]).toMatchObject({ value: "drc", labelKey: "switch.drc.label" });
    expect(schema?.extras.bluetoothHal[0].value).toBe("offload");
    expect(schema?.extras.tools.bluetoothHal).toEqual({ available: false, operations: { status: true, set: false, reset: false } });
    expect(schema?.extras.tools.usbPeriod?.operations.reset).toBe(false);
    expect(schema?.extras.resamplerPresets[0].group).toBe("resampler.group.standard.label");
    expect(schema?.extras.resamplerPresets.at(-1)).toMatchObject({ value: "custom", labelKey: "resampler.custom.label" });
    expect(schema?.extras.resamplerBypass[0].value).toBe("none");
    expect(schema?.extras.jitterFeatures[0]).toMatchObject({ value: "selinux", highRisk: true });
    expect(schema?.extras.jitterFeatures[1].capabilities?.io_parameters).toBe(true);
    expect(schema?.extras.jitterFeatures[2].requiresAudioRestart).toBe(true);
    expect(schema?.extras.ioSchedulers[0].labelKey).toBe("jitter.io_scheduler.*.label");
    expect(schema?.extras.diagnosticsCompleteOutput).toBe(false);
    expect(schema?.extras.bluetoothHalDefault).toBe("offload");
    expect(schema?.extras.resamplerDefaultPreset).toBe("179-408-99");
    expect(schema?.extras.resamplerCustomDefault).toEqual({ bypass: "none", cheat: true, stopBand: 179, halfLength: 408, percent: 99 });
    expect(schema?.extras.usbPeriodDefault).toBe(2250);
    expect(schema?.extras.diagnosticDefault).toBe("audio");
    expect(schema?.extras.jitterDefaults).toMatchObject({ ioScheduler: "*", ioTone: "medium", wifiNoRestart: false });
    expect(schema?.extras.jitterResetFeatures).toEqual(["selinux", "io"]);
    expect(schema?.extras.jitterWifiNoRestart).toBe(false);
  });

  it("rejects unsupported or incomplete contracts", () => {
    expect(parseControllerSchema(valid.replace('"schema_version":1', '"schema_version":2'))).toBeUndefined();
    expect(parseControllerSchema("not json")).toBeUndefined();
  });
});
