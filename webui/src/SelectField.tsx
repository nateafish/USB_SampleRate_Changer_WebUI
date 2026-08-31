import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { CheckIcon } from "./Icons";
import type { Language } from "./i18n";

export type SelectOption = readonly [value: string, label: string];
export type SelectGroup = {
  label: string;
  options: ReadonlyArray<SelectOption>;
};

export default function SelectField(props: {
  id?: string;
  title: string;
  value: string;
  options: ReadonlyArray<SelectOption>;
  groups?: ReadonlyArray<SelectGroup>;
  language?: Language;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [closing, setClosing] = createSignal(false);
  const allOptions = createMemo(() => props.groups?.flatMap((group) => group.options) ?? props.options);
  const selectedLabel = createMemo(() => allOptions().find(([value]) => value === props.value)?.[1] ?? props.value);
  const selectedTitle = createMemo(() => selectedLabel().split("\n", 1)[0]);
  const historyKey = `select-${Math.random().toString(36).slice(2)}`;
  let optionList: HTMLDivElement | undefined;
  let ownsHistoryEntry = false;
  let closeTimer: number | undefined;
  const closeLabel = () => props.language === "en" ? `Close ${props.title} selector` : props.title;

  function openDialog() {
    if (open()) return;
    setClosing(false);
    window.history.pushState({ ...window.history.state, usbSrOverlay: historyKey }, "");
    ownsHistoryEntry = true;
    setOpen(true);
  }

  function finishClose() {
    window.clearTimeout(closeTimer);
    setOpen(false);
    setClosing(false);
  }

  function closeDialog(fromHistory = false) {
    if (!open() || closing()) return;
    setClosing(true);
    if (!fromHistory && ownsHistoryEntry && window.history.state?.usbSrOverlay === historyKey) {
      window.history.back();
    }
    ownsHistoryEntry = false;
    window.clearTimeout(closeTimer);
    closeTimer = window.setTimeout(finishClose, 190);
  }

  onMount(() => {
    const handlePopState = () => {
      if (!open()) return;
      closeDialog(true);
    };
    window.addEventListener("popstate", handlePopState);
    onCleanup(() => {
      window.removeEventListener("popstate", handlePopState);
      window.clearTimeout(closeTimer);
    });
  });

  createEffect(() => {
    if (!open()) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeDialog();
    };
    document.addEventListener("keydown", closeOnEscape);
    const frame = window.requestAnimationFrame(() => {
      optionList?.querySelector<HTMLElement>(".select-option.selected")?.scrollIntoView({ block: "center" });
    });
    onCleanup(() => {
      document.removeEventListener("keydown", closeOnEscape);
      window.cancelAnimationFrame(frame);
    });
  });

  function renderOption([value, label]: SelectOption) {
    const [title, ...details] = label.split("\n");
    return <button
      type="button"
      role="option"
      class="select-option"
      classList={{ selected: value === props.value }}
      aria-selected={value === props.value}
      onClick={() => { props.onChange(value); closeDialog(); }}
    >
      <span class="select-option-copy"><span>{title}</span><Show when={details.length > 0}><small>{details.join("\n")}</small></Show></span><span class="select-check"><Show when={value === props.value}><CheckIcon /></Show></span>
    </button>;
  }

  return (
    <>
      <button
        id={props.id}
        type="button"
        class="select-trigger"
        aria-haspopup="listbox"
        aria-expanded={open() && !closing()}
        disabled={props.disabled}
        onClick={openDialog}
      >
        <span>{selectedTitle()}</span><span class="select-chevron" aria-hidden="true" />
      </button>
      <Show when={open()}>
        <Portal>
          <div class="select-dialog-layer" classList={{ closing: closing() }} data-no-page-drag role="presentation">
            <button class="select-dialog-backdrop" aria-label={closeLabel()} onClick={() => closeDialog()} />
            <section
              class="select-dialog"
              role="dialog"
              aria-modal="true"
              aria-label={props.title}
              onAnimationEnd={(event) => {
                if (closing() && event.target === event.currentTarget) finishClose();
              }}
            >
              <header><h2>{props.title}</h2><button type="button" class="dialog-close" aria-label={closeLabel()} onClick={() => closeDialog()}>×</button></header>
              <div class="select-option-list" role="listbox" aria-label={props.title} ref={optionList}>
                <Show when={props.groups} fallback={<For each={props.options}>{(option) => renderOption(option)}</For>}>
                  <For each={props.groups}>{(group) => <section class="select-option-group"><h3>{group.label}</h3><For each={group.options}>{(option) => renderOption(option)}</For></section>}</For>
                </Show>
              </div>
            </section>
          </div>
        </Portal>
      </Show>
    </>
  );
}
