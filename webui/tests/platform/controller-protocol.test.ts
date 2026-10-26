import { describe, expect, it } from "vitest";
import { diagnosticText } from "../../src/platform/controller-protocol";

describe("diagnosticText", () => {
  it("removes controller execution metadata without dropping diagnostics", () => {
    const text = diagnosticText({
      code: 0,
      stdout: [
        "controller_action=extra-diagnose-audio",
        "operation_kind=query",
        "operation_result=success",
        "operation_applied=0",
        "controller_exit=0",
        "AudioFlinger state: running",
        "sample_rate=48000"
      ].join("\n"),
      stderr: "controller_upstream_started=1\nglobal namespace verified: mnt:[1]\nreal diagnostic warning"
    });
    expect(text).toBe("AudioFlinger state: running\nsample_rate=48000\n[stderr]\nreal diagnostic warning");
  });

  it("removes reapply markers if a shared result is displayed as diagnostics", () => {
    expect(diagnosticText({
      code: 1,
      stdout: "batch_step_started=1\nbatch_step_exit=1:0\nactual payload",
      stderr: ""
    })).toBe("actual payload");
  });
});
