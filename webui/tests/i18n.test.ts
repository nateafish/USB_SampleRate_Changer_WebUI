import { describe, expect, it } from "vitest";
import { translate, translateRuntime } from "../src/i18n";
import { english } from "../src/locales/en";
import { chinese } from "../src/locales/zh-CN";

describe("semantic translations", () => {
  it("resolves semantic keys in both languages", () => {
    expect(translate("zh-CN", "common.apply")).toBe("应用");
    expect(translate("en", "common.apply")).toBe("Apply");
    expect(translate("en", "bluetooth_hal.option.offload.label")).toBe("Hardware offload (recommended)");
    expect(translate("zh-CN", "switch.force_usbv2.label")).toBe("强制 USBv2 HAL");
    expect(translate("en", "jitter.io_scheduler.*.label")).toBe("Automatic");
    expect(translate("zh-CN", "resampler.custom.label")).toBe("自定义完整参数");
    expect(translate("en", "resampler.bypass.48.label")).toBe("From 48 kHz");
    expect(translate("zh-CN", "resampler.bypass.96.label")).toBe("96 kHz 起");
    expect(translate("zh-CN", "resampler.preset.194-520-100.label")).toBe("1:1 Bit-perfect");
    expect(translate("en", "resampler.preset.194-520-100.description")).toBe("194dB / 520 / 100%(cutoff)");
  });

  it("interpolates structured parameters without sentence regex matching", () => {
    expect(translateRuntime("zh-CN", "errors.operationFailed", { code: 7 })).toBe("执行失败，退出码 7");
    expect(translateRuntime("en", "errors.operationFailed", { code: 7 })).toBe("Operation failed, exit code 7");
    expect(translateRuntime("en", "unstructured backend output 7")).toBe("unstructured backend output 7");
  });

  it("localizes differentiated controller outcomes in both languages", () => {
    expect(translate("zh-CN", "errors.controller.partiallyApplied"))
      .toContain("部分修改已经应用");
    expect(translate("en", "errors.controller.partiallyApplied"))
      .toContain("some changes were applied");
    expect(translate("zh-CN", "errors.controller.appliedPersistFailed"))
      .toContain("配置保存失败");
    expect(translate("en", "errors.controller.queryTimedOut"))
      .toContain("No system changes were made");
  });

  it("uses English as the canonical source language", () => {
    expect(translate("en", "runtime.spawnUnavailable"))
      .toBe("KernelSU/APatch WebUI spawn interface is unavailable");
    expect(translate("en", "unknown.runtime.message"))
      .toBe("unknown.runtime.message");
    expect(Object.keys(chinese).sort()).toEqual(Object.keys(english).sort());
    expect(Object.values(english).some((value) => /\p{Script=Han}/u.test(value))).toBe(false);
  });

  it("covers controller schema action labels in both languages", () => {
    for (const value of ["aosp", "legacy", "offload", "sysbta"]) {
      const key = `bluetooth_hal.action.${value}.label`;
      expect(translate("en", key)).not.toBe(key);
      expect(translate("zh-CN", key)).not.toBe(key);
    }
  });

  it("keeps placeholders aligned across locales", () => {
    const placeholders = (value: string) => [...value.matchAll(/\{([a-zA-Z0-9_]+)\}/g)]
      .map((match) => match[1])
      .sort();
    for (const key of Object.keys(english) as Array<keyof typeof english>) {
      expect(placeholders(chinese[key]), key).toEqual(placeholders(english[key]));
    }
  });

  it("does not retain the removed system-default resampler option", () => {
    expect(Object.keys(english).some((key) => key.startsWith("resampler.preset.default"))).toBe(false);
    expect(Object.keys(chinese).some((key) => key.startsWith("resampler.preset.default"))).toBe(false);
  });
});
