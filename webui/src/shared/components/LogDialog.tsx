import { Show } from "solid-js";
import type { Translator } from "../types";

export function LogDialog(props: { open: boolean; log: string; tx: Translator; onClose: () => void }) {
  return (
    <Show when={props.open}>
      <div class="dialog-layer" role="presentation">
        <button class="dialog-backdrop" aria-label={props.tx("dialog.logs.close")} onClick={props.onClose} />
        <section class="log-dialog" role="dialog" aria-modal="true" aria-labelledby="log-title">
          <header><div><h2 id="log-title">{props.tx("app.logs.title")}</h2></div><button class="icon-button" aria-label={props.tx("dialog.logs.close")} onClick={props.onClose}>×</button></header>
          <pre>{props.log || props.tx("dialog.logs.empty")}</pre>
        </section>
      </div>
    </Show>
  );
}
