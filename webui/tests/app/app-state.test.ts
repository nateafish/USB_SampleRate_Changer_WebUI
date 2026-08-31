import { createRoot, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";
import { createOverlayManager } from "../../src/app/createOverlayManager";
import { createPageNavigation } from "../../src/app/createPageNavigation";

describe("app navigation and overlays", () => {
  it("projects page activation into history without adding a Back entry", () => {
    createRoot((dispose) => {
      window.history.replaceState({}, "");
      const replaceState = vi.spyOn(window.history, "replaceState");
      const navigation = createPageNavigation({ browserWindow: window });
      navigation.activatePage("tools");
      expect(navigation.activePage()).toBe("tools");
      expect(navigation.pageProgress()).toBe(1);
      expect(replaceState).toHaveBeenLastCalledWith(expect.objectContaining({ usbSrPage: "tools" }), "");
      replaceState.mockRestore();
      dispose();
    });
  });

  it("gives confirmation Back handling priority over other overlays", () => {
    window.history.replaceState({}, "");
    const host = document.createElement("div");
    let overlays!: ReturnType<typeof createOverlayManager>;
    let setConfirmOpen!: (value: boolean) => void;
    const dismissed = vi.fn(() => setConfirmOpen(false));
    const dispose = render(() => {
      const [confirmOpen, setOpen] = createSignal(false);
      setConfirmOpen = setOpen;
      overlays = createOverlayManager({
        activePage: () => "policy",
        confirmOpen,
        onConfirmDismiss: dismissed,
        browserWindow: window
      });
      return null;
    }, host);
    overlays.openOverlay("log");
    setConfirmOpen(true);
    overlays.openConfirmHistory();
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(dismissed).toHaveBeenCalledOnce();
    expect(overlays.logOpen()).toBe(true);
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(overlays.logOpen()).toBe(false);
    dispose();
  });
});
