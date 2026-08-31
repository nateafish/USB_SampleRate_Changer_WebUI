import { describe, expect, it } from "vitest";
import { ControllerClient } from "../../src/platform/controller-client";
import { defaultPolicySettings } from "../../src/domain/policy";

describe("ControllerClient typed methods", () => {
  it("reads the controller's persisted execution log", async () => {
    const commands: string[] = [];
    const client = new ControllerClient(async (command) => {
      commands.push(command);
      return { code: 0, stdout: "persisted log", stderr: "" };
    }, "/ctl");
    await expect(client.logs()).resolves.toMatchObject({ code: 0, stdout: "persisted log" });
    expect(commands).toEqual(["'/ctl' 'logs'"]);
  });

  it("keeps CLI argument construction behind the platform boundary", async () => {
    const commands: string[] = [];
    const client = new ControllerClient(async (command) => {
      commands.push(command);
      return { code: 0, stdout: "", stderr: "" };
    }, "/ctl");

    await client.apply({ ...defaultPolicySettings(), rate: "custom", customRate: "123456", drc: true });
    await client.setBluetoothHal("legacy");
    await client.applyResampler({ preset: "custom", bypass: "96", cheat: false, stopBand: 194, halfLength: 520, percent: 42 });
    await client.setUsbPeriod(2250);
    await client.diagnose("alsa", true);
    await client.setJitter({ feature: "io", enabled: true, ioScheduler: "bfq", ioTone: "boost" });
    await client.setJitter({ feature: "wifi", enabled: true, wifiNoRestart: true });

    expect(commands).toEqual([
      "'/ctl' 'apply' '--policy' 'auto' '--sample-rate' '123456' '--bit-depth' '32' '--drc'",
      "'/ctl' 'extra' 'bluetooth-hal' 'legacy'",
      "'/ctl' 'extra' 'resampler' 'custom' '96' 'cutoff' '194' '520' '42'",
      "'/ctl' 'extra' 'usb-period' '2250'",
      "'/ctl' 'extra' 'diagnose' 'alsa' 'all'",
      "'/ctl' 'extra' 'jitter' 'enable' 'io' 'bfq' 'boost'",
      "'/ctl' 'extra' 'jitter' 'enable' 'wifi' 'no-restart'"
    ]);
  });

  it("quotes apostrophes passed through the generic compatibility method", async () => {
    let command = "";
    const client = new ControllerClient(async (value) => {
      command = value;
      return { code: 0, stdout: "", stderr: "" };
    }, "/ctl");
    await client.extra(["diagnose", "a'b"]);
    expect(command).toBe("'/ctl' 'extra' 'diagnose' 'a'\"'\"'b'");
  });

  it("keeps query results separate from mutation progress", async () => {
    const outputs = [
      "operation_kind=query\noperation_result=success\noperation_applied=0\n",
      "operation_kind=mutation\noperation_result=failed\noperation_applied=1\noperation_state=applied\n"
    ];
    const client = new ControllerClient(async () => ({
      code: 0,
      stdout: outputs.shift() ?? "",
      stderr: ""
    }), "/ctl");

    await expect(client.diagnose("audio", false)).resolves.toMatchObject({
      operation: { kind: "query", result: "success", applied: false, state: undefined }
    });
    await expect(client.reset()).resolves.toMatchObject({
      operation: { kind: "mutation", result: "failed", applied: true, state: "applied" }
    });
  });
});
