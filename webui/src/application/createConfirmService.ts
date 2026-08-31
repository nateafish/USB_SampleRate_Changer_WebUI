import { createSignal, getOwner, onCleanup, type Accessor } from "solid-js";
import type { ConfirmRequest } from "../shared/types";

export type ConfirmOptions = {
  title: string;
  message: string;
  confirmLabel?: string;
  showCancel?: boolean;
};

export type ConfirmCloseReason = "accepted" | "cancelled" | "superseded" | "disposed";

export type ConfirmServiceOptions = {
  translate?: (value: string) => string;
  translateMessage?: (value: string) => string;
  onOpen?: () => void;
  onClose?: (reason: ConfirmCloseReason, external: boolean) => void;
};

export type ConfirmService = {
  request: Accessor<ConfirmRequest | undefined>;
  confirm: (options: ConfirmOptions) => Promise<boolean>;
  accept: (external?: boolean) => void;
  cancel: (external?: boolean) => void;
  dispose: () => void;
};

type PendingConfirmation = {
  ticket: symbol;
  resolve: (accepted: boolean) => void;
};

/**
 * Converts the callback-oriented ConfirmDialog contract into a safe Promise.
 * Stale dialog callbacks cannot resolve a newer request and every request is
 * resolved exactly once, including supersession, back navigation and cleanup.
 */
export function createConfirmService(options: ConfirmServiceOptions = {}): ConfirmService {
  const [request, setRequest] = createSignal<ConfirmRequest>();
  const translate = options.translate ?? ((value: string) => value);
  const translateMessage = options.translateMessage ?? translate;
  let pending: PendingConfirmation | undefined;
  let disposed = false;

  function settle(
    ticket: symbol,
    accepted: boolean,
    reason: ConfirmCloseReason,
    external: boolean
  ) {
    if (!pending || pending.ticket !== ticket) return;
    const current = pending;
    pending = undefined;
    setRequest(undefined);
    current.resolve(accepted);
    options.onClose?.(reason, external);
  }

  function settleCurrent(
    accepted: boolean,
    reason: ConfirmCloseReason,
    external = false
  ) {
    if (pending) settle(pending.ticket, accepted, reason, external);
  }

  function confirm(confirmOptions: ConfirmOptions): Promise<boolean> {
    if (disposed) return Promise.resolve(false);
    settleCurrent(false, "superseded", true);

    return new Promise((resolve) => {
      const ticket = Symbol("confirmation");
      pending = { ticket, resolve };
      setRequest({
        title: translate(confirmOptions.title),
        message: translateMessage(confirmOptions.message),
        confirmLabel: translate(confirmOptions.confirmLabel ?? "common.continue"),
        showCancel: confirmOptions.showCancel,
        action: () => settle(ticket, true, "accepted", false),
        cancel: () => settle(ticket, false, "cancelled", false)
      });
      options.onOpen?.();
    });
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    settleCurrent(false, "disposed", true);
  }

  if (getOwner()) onCleanup(dispose);

  return {
    request,
    confirm,
    accept: (external = false) => settleCurrent(true, "accepted", external),
    cancel: (external = false) => settleCurrent(false, "cancelled", external),
    dispose
  };
}
