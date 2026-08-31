import { Show } from "solid-js";
import { WarningIcon } from "../../Icons";
import type { ConfirmRequest, Translator } from "../types";

export function ConfirmDialog(props: {
  request?: ConfirmRequest;
  tx: Translator;
  onDismiss: () => void;
  onAccept: () => void;
}) {
  return (
    <Show when={props.request} keyed>{(request) =>
      <div class="dialog-layer confirm-layer" data-no-page-drag role="presentation">
        <button class="dialog-backdrop" aria-label={props.tx("dialog.confirm.cancel")} onClick={props.onDismiss} />
        <section class="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-message">
          <div class="confirm-mark" aria-hidden="true"><WarningIcon /></div>
          <div><h2 id="confirm-title">{request.title}</h2><p id="confirm-message">{request.message}</p></div>
          <div class="confirm-actions" classList={{ single: request.showCancel === false }}>
            <Show when={request.showCancel !== false}><button type="button" class="secondary-button" onClick={props.onDismiss}>{props.tx("common.cancel")}</button></Show>
            <button type="button" class="danger-button" onClick={props.onAccept}>{request.confirmLabel}</button>
          </div>
        </section>
      </div>
    }</Show>
  );
}
