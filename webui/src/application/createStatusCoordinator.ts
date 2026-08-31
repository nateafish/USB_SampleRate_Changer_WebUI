import { createSignal, type Accessor } from "solid-js";

type StatusExecResult = {
  code: number;
  stdout: string;
  stderr: string;
};

export type StatusResponse<TStatus> = {
  result: StatusExecResult;
  status: TStatus;
};

export type StatusHydrator<TStatus> = (status: TStatus) => void;
export type StatusScope = "policy" | "tools" | "tuning" | "settings";
export type StatusScopeSelection<TScope extends string> = "all" | TScope | readonly TScope[];

export type StatusCoordinatorOptions<TStatus, TScope extends string = StatusScope> = {
  initialStatus?: TStatus;
  fetch: () => Promise<StatusResponse<TStatus>>;
  hydrators: Partial<Record<TScope, StatusHydrator<TStatus>>>;
  hydrateAlways?: StatusHydrator<TStatus>;
  failureMessage?: string;
  onError?: (error: unknown) => void;
  onSuccess?: (status: TStatus, scope: StatusScopeSelection<TScope>) => void;
};

export type StatusCoordinator<TStatus, TScope extends string = StatusScope> = {
  status: Accessor<TStatus | undefined>;
  read: () => Promise<TStatus>;
  hydrate: (status: TStatus, scope?: StatusScopeSelection<TScope>) => void;
  refresh: (scope?: StatusScopeSelection<TScope>) => Promise<TStatus>;
  tryRefresh: (scope?: StatusScopeSelection<TScope>) => Promise<boolean>;
  replaceStatus: (status: TStatus) => void;
};

function statusError(result: StatusExecResult, fallback: string): Error {
  return new Error(result.stderr || result.stdout || fallback);
}

/**
 * Fetches the platform-normalized status and applies only the requested
 * feature hydrators. It deliberately owns no busy lock so it can be used as
 * an inner step of an already-exclusive mutation without deadlocking.
 */
export function createStatusCoordinator<TStatus, TScope extends string = StatusScope>(
  options: StatusCoordinatorOptions<TStatus, TScope>
): StatusCoordinator<TStatus, TScope> {
  const [status, setStatus] = createSignal<TStatus | undefined>(options.initialStatus);

  function replaceStatus(next: TStatus) {
    setStatus(() => next);
  }

  async function read(): Promise<TStatus> {
    const response = await options.fetch();
    if (response.result.code !== 0) {
      throw statusError(response.result, options.failureMessage ?? "app.status.readFailed");
    }
    replaceStatus(response.status);
    return response.status;
  }

  function hydrate(next: TStatus, scope: StatusScopeSelection<TScope> = "all") {
    options.hydrateAlways?.(next);
    if (scope === "all") {
      for (const apply of Object.values(options.hydrators) as Array<StatusHydrator<TStatus> | undefined>) {
        apply?.(next);
      }
      return;
    }

    const scopes = (typeof scope === "string" ? [scope] : scope) as readonly TScope[];
    for (const selected of scopes) options.hydrators[selected]?.(next);
  }

  async function refresh(scope: StatusScopeSelection<TScope> = "all"): Promise<TStatus> {
    const next = await read();
    hydrate(next, scope);
    options.onSuccess?.(next, scope);
    return next;
  }

  async function tryRefresh(scope: StatusScopeSelection<TScope> = "all"): Promise<boolean> {
    try {
      await refresh(scope);
      return true;
    } catch (error) {
      options.onError?.(error);
      return false;
    }
  }

  return { status, read, hydrate, refresh, tryRefresh, replaceStatus };
}
