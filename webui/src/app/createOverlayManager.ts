import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import type { PageId } from "./createPageNavigation";

export type OverlayName = "policy-help" | "log";

export type OverlayManagerOptions = {
  activePage: Accessor<PageId>;
  confirmOpen: Accessor<boolean>;
  browserWindow?: Window;
  /** Invoked whenever a confirmation is cancelled, including Back and cleanup. */
  onConfirmDismiss?: () => void;
};

export type OverlayManager = {
  policyHelpOpen: Accessor<boolean>;
  logOpen: Accessor<boolean>;
  openOverlay: (name: OverlayName) => void;
  closeOverlay: (name: OverlayName, fromHistory?: boolean) => void;
  openConfirmHistory: () => void;
  closeConfirmHistory: (fromHistory?: boolean) => void;
};

/**
 * Owns modal state and maps each opened modal to one browser-history entry.
 * Popstate closes only the top-priority visible overlay, matching Android Back
 * behavior when a confirmation is displayed over another dialog.
 */
export function createOverlayManager(options: OverlayManagerOptions): OverlayManager {
  const browserWindow = options.browserWindow ?? window;
  const [policyHelpOpen, setPolicyHelpOpen] = createSignal(false);
  const [logOpen, setLogOpen] = createSignal(false);

  function pushOverlayHistory(name: OverlayName | "confirm") {
    browserWindow.history.pushState({
      ...browserWindow.history.state,
      usbSrPage: options.activePage(),
      usbSrOverlay: name
    }, "");
  }

  function openOverlay(name: OverlayName) {
    pushOverlayHistory(name);
    if (name === "policy-help") setPolicyHelpOpen(true);
    else setLogOpen(true);
  }

  function closeOverlay(name: OverlayName, fromHistory = false) {
    if (name === "policy-help") setPolicyHelpOpen(false);
    else setLogOpen(false);
    if (!fromHistory && browserWindow.history.state?.usbSrOverlay === name) {
      browserWindow.history.back();
    }
  }

  function openConfirmHistory() {
    pushOverlayHistory("confirm");
  }

  function closeConfirmHistory(fromHistory = false) {
    if (!fromHistory && browserWindow.history.state?.usbSrOverlay === "confirm") {
      browserWindow.history.back();
    }
  }

  function handlePopState() {
    if (options.confirmOpen()) {
      options.onConfirmDismiss?.();
      return;
    }
    if (policyHelpOpen()) {
      closeOverlay("policy-help", true);
      return;
    }
    if (logOpen()) closeOverlay("log", true);
  }

  onMount(() => browserWindow.addEventListener("popstate", handlePopState));

  onCleanup(() => {
    browserWindow.removeEventListener("popstate", handlePopState);
    if (options.confirmOpen()) options.onConfirmDismiss?.();
  });

  return {
    policyHelpOpen,
    logOpen,
    openOverlay,
    closeOverlay,
    openConfirmHistory,
    closeConfirmHistory
  };
}
