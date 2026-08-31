import type { ExecResult } from "../domain/models";
import { LocalizedError, type TranslationParams } from "../shared/types";
import type { ControllerOperationMetadata } from "./controller-protocol";
import { outputText } from "./controller-protocol";

export type OperationAwareExecResult = ExecResult & {
  operation?: ControllerOperationMetadata;
};

/**
 * Converts the Controller's orthogonal result/effect fields into a concise
 * user-facing conclusion. Full stdout/stderr remains available in the shared
 * execution log; legacy Controllers still fall back to their raw output.
 */
export function controllerExecutionError(
  result: OperationAwareExecResult,
  fallbackKey: string,
  fallbackParams: TranslationParams = { code: result.code }
): Error {
  const operation = result.operation;
  if (!operation?.kind || !operation.result) {
    const raw = outputText(result);
    return raw ? new Error(raw) : new LocalizedError(fallbackKey, fallbackParams);
  }

  if (operation.kind === "query") {
    if (operation.result === "timed_out") {
      return new LocalizedError("errors.controller.queryTimedOut");
    }
    if (operation.result === "not_started") {
      return new LocalizedError("errors.controller.queryNotStarted");
    }
    return new LocalizedError("errors.controller.queryFailed", { code: result.code });
  }

  if (operation.stateDegraded) {
    return new LocalizedError("errors.controller.stateDegraded");
  }
  if (operation.state === "partially_applied") {
    return new LocalizedError("errors.controller.partiallyApplied");
  }
  if (operation.state === "possibly_applied") {
    return new LocalizedError(
      operation.result === "timed_out"
        ? "errors.controller.possiblyAppliedTimedOut"
        : "errors.controller.possiblyApplied"
    );
  }
  if (operation.state === "not_started" || operation.result === "not_started") {
    return new LocalizedError("errors.controller.mutationNotStarted");
  }
  if (operation.state === "applied" && operation.statePersist === "failed") {
    return new LocalizedError("errors.controller.appliedPersistFailed");
  }
  if (operation.state === "applied") {
    return new LocalizedError("errors.controller.appliedButFailed");
  }
  if (operation.result === "timed_out") {
    return new LocalizedError("errors.controller.mutationTimedOut");
  }
  return new LocalizedError(fallbackKey, fallbackParams);
}
