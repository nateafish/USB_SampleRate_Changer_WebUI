// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { executeRoot } from "../../src/platform/ksu-runtime";

describe("root spawn stream assembly", () => {
  afterEach(() => {
    delete window.ksu;
  });

  it("restores newlines between APatch/KernelSU line events", async () => {
    window.ksu = {
      spawn: vi.fn((_command, _args, _options, callbackRef) => {
        const child = (window as unknown as Record<string, { stdout: { emit: (event: "data", data: string) => void }; stderr: { emit: (event: "data", data: string) => void }; emit: (event: "exit", code: number) => void }>)[String(callbackRef)];
        child.stdout.emit("data", '{"key":"value"}');
        child.stdout.emit("data", "next=1");
        child.stderr.emit("data", "warn");
        child.stderr.emit("data", "detail");
        child.emit("exit", 0);
      })
    };

    await expect(executeRoot("controller schema --json")).resolves.toEqual({
      code: 0,
      stdout: '{"key":"value"}\nnext=1',
      stderr: "warn\ndetail"
    });
  });
});
