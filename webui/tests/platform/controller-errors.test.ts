import { describe, expect, it } from "vitest";
import { LocalizedError } from "../../src/shared/types";
import { controllerExecutionError } from "../../src/platform/controller-errors";

function keyOf(error: Error): string {
  expect(error).toBeInstanceOf(LocalizedError);
  return (error as LocalizedError).key;
}

describe("controllerExecutionError", () => {
  it("distinguishes mutation side-effect states", () => {
    const base = { code: 1, stdout: "details", stderr: "" };
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "mutation", result: "failed", state: "not_started" }
    }, "fallback"))).toBe("errors.controller.mutationNotStarted");
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "mutation", result: "failed", state: "partially_applied" }
    }, "fallback"))).toBe("errors.controller.partiallyApplied");
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "mutation", result: "timed_out", state: "possibly_applied" }
    }, "fallback"))).toBe("errors.controller.possiblyAppliedTimedOut");
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "mutation", result: "failed", state: "applied", statePersist: "failed" }
    }, "fallback"))).toBe("errors.controller.appliedPersistFailed");
  });

  it("distinguishes degraded preflight and query failures", () => {
    const base = { code: 2, stdout: "details", stderr: "" };
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "mutation", result: "not_started", state: "not_started", stateDegraded: true }
    }, "fallback"))).toBe("errors.controller.stateDegraded");
    expect(keyOf(controllerExecutionError({
      ...base,
      operation: { kind: "query", result: "timed_out" }
    }, "fallback"))).toBe("errors.controller.queryTimedOut");
  });

  it("keeps raw output fallback for legacy Controllers", () => {
    expect(controllerExecutionError({ code: 1, stdout: "legacy failure", stderr: "" }, "fallback").message)
      .toBe("legacy failure");
  });
});
