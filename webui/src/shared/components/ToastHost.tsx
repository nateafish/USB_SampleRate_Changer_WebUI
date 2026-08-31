import { For } from "solid-js";
import type { ToastItem } from "../types";

export function ToastHost(props: { toasts: ToastItem[] }) {
  return (
    <div id="toast-container" aria-live="polite">
      <For each={props.toasts}>{(toast) =>
        <div class={`toast${toast.tone ? ` ${toast.tone}` : ""}`}>{toast.message}</div>
      }</For>
    </div>
  );
}
