import { SectionHeading } from "../../shared/components/SectionHeading";
import type { ToolAction } from "../../domain/options";
import type { Translator } from "../../shared/types";
import type { NumericRange } from "../../platform/controller-schema";

export function UsbPeriodCard(props: {
  value: string;
  normalizedValue: string;
  busy: boolean;
  canApply: boolean;
  canReset: boolean;
  toolAction?: ToolAction;
  tx: Translator;
  limit: NumericRange;
  onInput: (value: string) => void;
  onBlur: () => void;
  onStep: (direction: -1 | 1) => void;
  onReset: () => void;
  onApply: () => void;
}) {
  function keepRangeThumbOnly(event: PointerEvent) {
    const input = event.currentTarget as HTMLInputElement;
    delete input.dataset.blockedClick;
    delete input.dataset.blockedValue;
    const rect = input.getBoundingClientRect();
    const min = Number(input.min);
    const max = Number(input.max);
    const value = Number(input.value);
    if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min || !Number.isFinite(value)) return;
    const inset = Math.min(12, rect.height / 2);
    const fraction = Math.max(0, Math.min(1, (value - min) / (max - min)));
    const thumbCenter = rect.left + inset + fraction * Math.max(0, rect.width - inset * 2);
    if (Math.abs(event.clientX - thumbCenter) > Math.max(14, inset * 1.6)) {
      input.dataset.blockedClick = "true";
      input.dataset.blockedValue = input.value;
      event.preventDefault();
      event.stopPropagation();
    }
  }

  function handleRangeInput(event: InputEvent & { currentTarget: HTMLInputElement }) {
    const range = event.currentTarget;
    if (range.dataset.blockedClick === "true") {
      range.value = range.dataset.blockedValue ?? range.value;
      return;
    }
    props.onInput(range.value);
  }

  function releaseRangePointer(event: Event) {
    const range = event.currentTarget as HTMLInputElement;
    if (range.dataset.blockedClick !== "true") return;
    range.value = range.dataset.blockedValue ?? range.value;
    window.setTimeout(() => {
      delete range.dataset.blockedClick;
      delete range.dataset.blockedValue;
    }, 0);
  }

  function cancelBlockedRangeClick(event: MouseEvent) {
    const range = event.currentTarget as HTMLInputElement;
    if (range.dataset.blockedClick !== "true") return;
    event.preventDefault();
    event.stopPropagation();
    delete range.dataset.blockedClick;
    delete range.dataset.blockedValue;
  }

  return <article class="card tool-panel">
    <SectionHeading title={props.tx("tools.usbPeriod.title")} />
    <p class="field-help">{props.tx("tools.usbPeriod.description")}</p>
    <div class="period-control">
      <div class="period-stepper">
        <button type="button" data-no-page-drag aria-label={props.tx("tools.usbPeriod.decrease")} onClick={() => props.onStep(-1)} disabled={props.busy || !props.canApply || Number(props.normalizedValue) <= props.limit.min}>−</button>
        <label><input class="page-swipe-input" aria-label={props.tx("tools.usbPeriod.input")} inputmode="numeric" type="number" min={props.limit.min} max={props.limit.max} step={props.limit.step} value={props.value} disabled={props.busy || !props.canApply} onInput={(event) => props.onInput(event.currentTarget.value)} onBlur={props.onBlur} /><span>μs</span></label>
        <button type="button" data-no-page-drag aria-label={props.tx("tools.usbPeriod.increase")} onClick={() => props.onStep(1)} disabled={props.busy || !props.canApply || Number(props.normalizedValue) >= props.limit.max}>+</button>
      </div>
      <input class="period-range" data-no-page-drag aria-label={props.tx("tools.usbPeriod.range")} type="range" min={props.limit.min} max={props.limit.max} step={props.limit.step} value={props.normalizedValue} disabled={props.busy || !props.canApply} onPointerDown={keepRangeThumbOnly} onInput={handleRangeInput} onPointerUp={releaseRangePointer} onPointerCancel={releaseRangePointer} onClick={cancelBlockedRangeClick} />
      <div class="period-scale"><span>{props.limit.min} μs</span><strong>{props.normalizedValue} μs</strong><span>{props.limit.max} μs</span></div>
    </div>
    <div class="inline-actions"><button class="secondary-button" disabled={props.busy || !props.canReset} onClick={props.onReset}>{props.toolAction === "usb-period-reset" ? props.tx("common.working") : props.tx("common.reset")}</button><button class="primary-button" disabled={props.busy || !props.canApply} onClick={props.onApply}>{props.toolAction === "usb-period" ? props.tx("common.working") : props.tx("common.apply")}</button></div>
  </article>;
}
