import type { ExecResult } from "../domain/models";

declare global {
  interface Window {
    __usbSrCallbacks?: Record<string, (code: number, stdout: string, stderr: string) => void>;
    ksu?: {
      exec?: (command: string, options?: string, callback?: string) => void | string | Promise<unknown>;
      spawn?: (command: string, args: string, options?: string, callback?: string) => void;
    };
  }
}

let callbackSequence = 0;
let spawnSequence = 0;

type SpawnStream = {
  on: (event: "data", callback: (data: string) => void) => void;
  emit: (event: "data", data: string) => void;
};

type SpawnChild = {
  stdout: SpawnStream;
  stderr: SpawnStream;
  on: (event: "exit" | "error", callback: (value: number | Error) => void) => void;
  emit: (event: "exit" | "error", value: number | Error) => void;
};

function createSpawnStream(): SpawnStream {
  const listeners: Array<(data: string) => void> = [];
  return {
    on(event, callback) {
      if (event === "data") listeners.push(callback);
    },
    emit(event, data) {
      if (event === "data") listeners.forEach((callback) => callback(data));
    }
  };
}

function createSpawnChild(): SpawnChild {
  const listeners: Record<"exit" | "error", Array<(value: number | Error) => void>> = {
    exit: [],
    error: []
  };
  return {
    stdout: createSpawnStream(),
    stderr: createSpawnStream(),
    on(event, callback) {
      listeners[event].push(callback);
    },
    emit(event, value) {
      listeners[event].forEach((callback) => callback(value));
    }
  };
}

function rootSpawn(command: string): Promise<ExecResult> {
  return new Promise((resolve, reject) => {
    const ksu = window.ksu;
    if (!ksu || typeof ksu.spawn !== "function") {
      reject(new Error("runtime.spawnUnavailable"));
      return;
    }

    const id = `spawn${Date.now().toString(36)}_${(++spawnSequence).toString(36)}`;
    const callbackRef = `__usbSrSpawn_${id}`;
    const child = createSpawnChild();
    const callbacks = window as unknown as Record<string, unknown>;
    const stdout: string[] = [];
    const stderr: string[] = [];
    let settled = false;

    child.stdout.on("data", (data) => stdout.push(String(data ?? "")));
    child.stderr.on("data", (data) => stderr.push(String(data ?? "")));

    const cleanup = () => {
      delete callbacks[callbackRef];
      window.clearTimeout(timeout);
    };
    const fail = (error: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error instanceof Error ? error : new Error(String(error)));
    };

    child.on("exit", (code) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve({
        code: Number(code),
        // IMPORTANT: Do not change these to join(""). APatch/KernelSU expose
        // shell CallbackList elements as line events, not raw pipe chunks, and
        // strip each line ending before emitting "data". Without reinserting
        // newlines, adjacent schema/status records are silently concatenated.
        stdout: stdout.join("\n"),
        stderr: stderr.join("\n")
      });
    });
    child.on("error", fail);
    callbacks[callbackRef] = child;

    const timeout = window.setTimeout(() => {
      fail(new Error("runtime.timeout"));
    }, 120000);

    try {
      // KernelSU and APatch expect JSON-encoded args/options here. The
      // controller command remains shell-quoted for compatibility with both
      // spawn and the exec fallback.
      ksu.spawn(command, "[]", "{}", callbackRef);
    } catch (error) {
      fail(error);
    }
  });
}

function rootExec(command: string): Promise<ExecResult> {
  if (typeof window.ksu?.spawn === "function") return rootSpawn(command);
  return new Promise((resolve, reject) => {
    const ksu = window.ksu;
    if (!ksu || typeof ksu.exec !== "function") {
      reject(new Error("runtime.rootUnavailable"));
      return;
    }

    const id = `cb${Date.now().toString(36)}_${(++callbackSequence).toString(36)}`;
    const callbacks: Record<string, (code: number, stdout: string, stderr: string) => void> =
      window.__usbSrCallbacks ?? (window.__usbSrCallbacks = Object.create(null));
    const callbackRef = `window.__usbSrCallbacks.${id}`;
    const timeout = window.setTimeout(() => {
      delete callbacks[id];
      reject(new Error("runtime.timeout"));
    }, 120000);

    callbacks[id] = (code, stdout, stderr) => {
      window.clearTimeout(timeout);
      delete callbacks[id];
      resolve({ code: Number(code), stdout: String(stdout || ""), stderr: String(stderr || "") });
    };

    try {
      void ksu.exec(command, "{}", callbackRef);
    } catch (error) {
      window.clearTimeout(timeout);
      delete callbacks[id];
      reject(error);
    }
  });
}

export type RootExecutor = (command: string) => Promise<ExecResult>;

export const executeRoot: RootExecutor = rootExec;
