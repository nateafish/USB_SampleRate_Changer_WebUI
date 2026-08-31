import { createSignal, type Accessor } from "solid-js";

export type OperationId = string;

export type OperationRunResult<T> =
  | { started: true; value: T }
  | { started: false };

export type OperationCoordinator = {
  activeOperation: Accessor<OperationId | undefined>;
  busy: Accessor<boolean>;
  isActive: (id: OperationId) => boolean;
  tryRun: <T>(id: OperationId, operation: () => Promise<T> | T) => Promise<OperationRunResult<T>>;
  runExclusive: <T>(id: OperationId, operation: () => Promise<T> | T) => Promise<T | undefined>;
};

/**
 * Owns the application's single global operation slot.
 *
 * A second invocation is rejected rather than queued. This matches button
 * semantics: repeated taps must not start another root command later. Nested
 * work such as a status refresh should be called directly inside `operation`
 * instead of trying to acquire the coordinator again.
 */
export function createOperationCoordinator(): OperationCoordinator {
  const [activeOperation, setActiveOperation] = createSignal<OperationId>();

  async function tryRun<T>(
    id: OperationId,
    operation: () => Promise<T> | T
  ): Promise<OperationRunResult<T>> {
    if (activeOperation() !== undefined) return { started: false };

    setActiveOperation(id);
    try {
      return { started: true, value: await operation() };
    } finally {
      setActiveOperation(undefined);
    }
  }

  async function runExclusive<T>(
    id: OperationId,
    operation: () => Promise<T> | T
  ): Promise<T | undefined> {
    const result = await tryRun(id, operation);
    return result.started ? result.value : undefined;
  }

  return {
    activeOperation,
    busy: () => activeOperation() !== undefined,
    isActive: (id) => activeOperation() === id,
    tryRun,
    runExclusive
  };
}
