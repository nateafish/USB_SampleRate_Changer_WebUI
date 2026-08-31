import { describe, expect, it } from "vitest";
import { deviceStatusFromControllerStatus, statusFlag } from "../../src/domain/device-status";
import { defaultPolicySettings, policySettingsFromStatus, validatePolicySettings } from "../../src/domain/policy";
import {
  normalizeResamplerHalfLength,
  normalizeResamplerPercent,
  normalizeResamplerStopBand,
  normalizeUsbPeriod,
  toolsSettingsFromStatus
} from "../../src/domain/tools";
import {
  buildJitterOperations,
  defaultTuningSettings,
  markJitterDirty,
  requiresHighRiskConfirmation,
  tuningSettingsFromStatus
} from "../../src/domain/tuning";

describe("domain normalization", () => {
  it("validates standard and custom policy sample rates", () => {
    expect(validatePolicySettings(defaultPolicySettings())).toEqual({ valid: true, sampleRate: 44100 });
    expect(validatePolicySettings({ ...defaultPolicySettings(), rate: "custom", customRate: "123456" })).toEqual({ valid: true, sampleRate: 123456 });
    expect(validatePolicySettings({ ...defaultPolicySettings(), rate: "custom", customRate: "44099" }).valid).toBe(false);
    expect(validatePolicySettings({ ...defaultPolicySettings(), rate: "custom", customRate: "768001" }).valid).toBe(false);
    expect(validatePolicySettings({ ...defaultPolicySettings(), rate: "custom", customRate: "44100.5" }).valid).toBe(false);
  });

  it("clamps and steps USB and resampler values", () => {
    expect(normalizeUsbPeriod("bad")).toBe(2250);
    expect(normalizeUsbPeriod("bad", { min: 250, max: 1000, step: 125 })).toBe(1000);
    expect(normalizeUsbPeriod(1)).toBe(125);
    expect(normalizeUsbPeriod(49999)).toBe(50000);
    expect(normalizeUsbPeriod(2310)).toBe(2250);
    expect(normalizeResamplerStopBand(999)).toBe(242);
    expect(normalizeResamplerHalfLength(413)).toBe(416);
    expect(normalizeResamplerHalfLength("bad", { min: 16, max: 320, step: 16 })).toBe(320);
    expect(normalizeResamplerPercent(150, false)).toBe(100);
    expect(normalizeResamplerPercent(150, true)).toBe(150);
  });

  it("hydrates typed tools state while retaining absent values", () => {
    const previous = toolsSettingsFromStatus({
      bluetooth_hal: "legacy",
      usb_period: "2500",
      resampler_stop_band: "194"
    });
    const next = toolsSettingsFromStatus({ diagnostic_all: "1" }, previous);
    expect(next.bluetoothHal).toBe("legacy");
    expect(next.usbPeriod).toBe(2500);
    expect(next.resampler.stopBand).toBe(194);
    expect(next.diagnosticAll).toBe(true);
    expect(toolsSettingsFromStatus(
      { resampler_preset: "default" },
      previous,
      undefined,
      "179-408-99"
    ).resampler.preset).toBe("179-408-99");
  });

  it("uses dynamic schema directories when recalibrating status", () => {
    const policy = policySettingsFromStatus(
      { sample_rate: "88200" },
      defaultPolicySettings(),
      new Set(["48000", "88200"])
    );
    expect(policy.rate).toBe("88200");

    const tuning = tuningSettingsFromStatus(
      { jitter_new_feature: "1" },
      defaultTuningSettings(),
      ["new_feature"]
    );
    expect(tuning.jitter).toEqual({ new_feature: true });
  });
});

describe("tuning operations", () => {
  it("deduplicates dirty features and builds typed extra arguments", () => {
    expect(markJitterDirty(["io"], "io")).toEqual(["io"]);
    const settings = defaultTuningSettings();
    settings.jitter.io = true;
    settings.jitter.wifi = true;
    settings.ioScheduler = "bfq";
    settings.ioTone = "boost";
    settings.wifiNoRestart = true;
    expect(buildJitterOperations(["io", "wifi"], settings)).toEqual([
      { feature: "io", enabled: true, ioScheduler: "bfq", ioTone: "boost" },
      { feature: "wifi", enabled: true, wifiNoRestart: true }
    ]);
  });

  it("only flags enabled SELinux and thermal operations as high risk", () => {
    expect(requiresHighRiskConfirmation({ feature: "selinux", enabled: true })).toBe(true);
    expect(requiresHighRiskConfirmation({ feature: "thermal", enabled: false })).toBe(false);
    expect(requiresHighRiskConfirmation({ feature: "doze", enabled: true })).toBe(false);
  });
});

describe("device status mapping", () => {
  it("converts controller flags and system fields to a typed snapshot", () => {
    const status = deviceStatusFromControllerStatus({
      sample_rate: "96000",
      bluetooth_a2dp_connected: "1",
      bluetooth_a2dp_state: "unknown",
      namespace_ok: "0",
      auto_reapply: "1",
      last_exit: "72",
      state_degraded: "1",
      state_degraded_reason: "state persist failed",
      state_migration: "unversioned->v3",
      state_recovery_reason: "ignored invalid sample_rate",
      last_operation_applied: "0",
      operation_state: "possibly_applied",
      last_operation_kind: "mutation",
      last_operation_result: "failed",
      last_operation_state: "partially_applied",
      last_state_persist: "failed",
      last_audit_status: "ok"
    });
    expect(status.policy.rate).toBe("96000");
    expect(status.system.a2dpConnected).toBe(false);
    expect(status.system.a2dpState).toBe("unknown");
    expect(status.system.namespaceOk).toBe(false);
    expect(status.autoReapply).toBe(true);
    expect(status.system.lastExit).toBe(72);
    expect(status.system.stateDegraded).toBe(true);
    expect(status.system.stateDegradedReason).toBe("state persist failed");
    expect(status.system.stateMigration).toBe("unversioned->v3");
    expect(status.system.stateRecoveryReason).toBe("ignored invalid sample_rate");
    expect(status.system.lastOperationApplied).toBe(false);
    expect(status.system.operationState).toBe("possibly_applied");
    expect(status.system.lastOperationKind).toBe("mutation");
    expect(status.system.lastOperationResult).toBe("failed");
    expect(status.system.lastOperationState).toBe("partially_applied");
    expect(status.system.lastStatePersist).toBe("failed");
    expect(status.system.lastAuditStatus).toBe("ok");
    expect(statusFlag(["1"])).toBe(false);
  });
});
