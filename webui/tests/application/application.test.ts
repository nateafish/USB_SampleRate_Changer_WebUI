import { describe, expect, it, vi } from "vitest";
import { createA2dpGuard } from "../../src/application/createA2dpGuard";
import { createConfirmService } from "../../src/application/createConfirmService";
import { createNotificationCenter } from "../../src/application/createNotificationCenter";
import { createOperationCoordinator } from "../../src/application/createOperationCoordinator";
import { createStatusCoordinator } from "../../src/application/createStatusCoordinator";
import { LocalizedError } from "../../src/shared/types";

describe("operation coordinator", () => {
  it("rejects concurrent work and always releases after failure", async () => {
    const coordinator = createOperationCoordinator();
    let release!: () => void;
    const first = coordinator.tryRun("first", () => new Promise<void>((resolve) => { release = resolve; }));
    expect(coordinator.busy()).toBe(true);
    await expect(coordinator.tryRun("second", () => undefined)).resolves.toEqual({ started: false });
    release();
    await first;
    expect(coordinator.busy()).toBe(false);

    await expect(coordinator.runExclusive("failure", () => { throw new Error("boom"); })).rejects.toThrow("boom");
    expect(coordinator.activeOperation()).toBeUndefined();
  });
});

describe("confirmation and notifications", () => {
  it("settles superseded confirmations exactly once", async () => {
    const service = createConfirmService();
    const first = service.confirm({ title: "one", message: "one" });
    const second = service.confirm({ title: "two", message: "two" });
    await expect(first).resolves.toBe(false);
    service.accept();
    await expect(second).resolves.toBe(true);
    expect(service.request()).toBeUndefined();
  });

  it("uses explicit tones and clears scheduled notifications", () => {
    const scheduled: Array<() => void> = [];
    const center = createNotificationCenter({
      createId: (() => { let id = 0; return () => String(++id); })(),
      schedule: (callback) => { scheduled.push(callback); return scheduled.length as ReturnType<typeof setTimeout>; },
      cancelSchedule: () => undefined
    });
    center.info("pending");
    center.success("done");
    center.error(new Error("failed"));
    expect(center.toasts().map(({ tone }) => tone)).toEqual([undefined, "success", "error"]);
    scheduled[1]?.();
    expect(center.toasts().map(({ message }) => message)).toEqual(["pending", "failed"]);
  });

  it("translates LocalizedError keys with structured parameters", () => {
    const center = createNotificationCenter({
      translate: (message, params) => `${message}:${params?.code ?? "none"}`,
      schedule: () => 1 as ReturnType<typeof setTimeout>,
      cancelSchedule: () => undefined
    });
    center.error(new LocalizedError("errors.operationFailed", { code: 9 }));
    expect(center.toasts()[0]).toMatchObject({
      message: "errors.operationFailed:9",
      tone: "error"
    });
  });
});

describe("status and A2DP services", () => {
  it("hydrates only selected status scopes", async () => {
    const hydrated: string[] = [];
    const coordinator = createStatusCoordinator<{ value: number }, "policy" | "tools">({
      fetch: async () => ({ result: { code: 0, stdout: "", stderr: "" }, status: { value: 2 } }),
      hydrators: {
        policy: () => hydrated.push("policy"),
        tools: () => hydrated.push("tools")
      },
      hydrateAlways: () => hydrated.push("always")
    });
    await coordinator.refresh("tools");
    expect(hydrated).toEqual(["always", "tools"]);
    expect(coordinator.status()).toEqual({ value: 2 });
  });

  it("recognizes exit 72 and opens Bluetooth settings after confirmation", async () => {
    const confirm = vi.fn().mockResolvedValue(true);
    const openBluetoothSettings = vi.fn().mockResolvedValue({ code: 0, stdout: "", stderr: "" });
    const success = vi.fn();
    const guard = createA2dpGuard({
      controller: {
        status: async () => ({ result: { code: 0, stdout: "", stderr: "" }, status: { bluetooth_a2dp_connected: "1" } }),
        openBluetoothSettings
      },
      confirmations: { confirm },
      notifications: { success, error: vi.fn() }
    });
    expect(await guard.confirmBeforeMutation()).toBe(true);
    expect(guard.isRouteFailure({ code: 72 })).toBe(true);
    expect(await guard.handleRouteFailure("设置")).toBe("settings-opened");
    expect(openBluetoothSettings).toHaveBeenCalledOnce();
    expect(success).toHaveBeenCalledOnce();
  });

  it("does not silently treat an unknown A2DP query as disconnected", async () => {
    const confirm = vi.fn().mockResolvedValue(false);
    const guard = createA2dpGuard({
      controller: {
        status: async () => ({
          result: { code: 0, stdout: "", stderr: "" },
          status: { bluetooth_a2dp_state: "unknown", bluetooth_a2dp_connected: "0" }
        }),
        openBluetoothSettings: vi.fn()
      },
      confirmations: { confirm },
      notifications: { success: vi.fn(), error: vi.fn() }
    });
    expect(await guard.confirmBeforeMutation()).toBe(false);
    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: "a2dp.unknown.title" }));
  });
});
