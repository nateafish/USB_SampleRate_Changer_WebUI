import { For, Show } from "solid-js";
import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import type { PolicySettings } from "../../domain/models";
import { displayRate, selectedRate } from "../../domain/policy";
import { SectionHeading } from "../../shared/components/SectionHeading";
import { ToggleRow } from "../../shared/components/ToggleRow";
import type { Translator } from "../../shared/types";
import type { PolicyPageModel } from "./createPolicyModel";
import type { SchemaOption } from "../../platform/controller-schema";
import type { NumericRange } from "../../platform/controller-schema";

export function PolicyPage(props: {
  model: PolicyPageModel;
  language: Language;
  tx: Translator;
  policyOptions: ReadonlyArray<SchemaOption>;
  rateOptions: ReadonlyArray<SchemaOption>;
  bitDepthOptions: ReadonlyArray<SchemaOption>;
  switchOptions: ReadonlyArray<SchemaOption>;
  sampleRateLimit: NumericRange;
  onOpenHelp: () => void;
}) {
  const label = (option: SchemaOption) => props.tx(option.labelKey);
  const rateLabel = (value: string) => {
    const option = props.rateOptions.find(({ value: optionValue }) => optionValue === value);
    return option ? label(option) : props.tx(displayRate(value));
  };
  const a2dpLabel = () => {
    const state = props.model.status().system.a2dpState;
    return state === "connected"
      ? props.tx("policy.status.connected")
      : state === "disconnected"
        ? props.tx("policy.status.disconnected")
        : props.tx("policy.status.unknown");
  };
  const policySelectOptions = () => props.policyOptions.map((option) => [option.value, label(option)] as const);
  const rateSelect = () => [...props.rateOptions.map((option) => [option.value, label(option)] as const), ["custom", props.tx("rate.custom")] as const];
  const bitSelect = () => props.bitDepthOptions.map((option) => [option.value, label(option)] as const);
  const switchBindings = {
    drc: "drc",
    force_usbv2: "forceUsbv2",
    force_bluetooth_qti: "forceBluetoothQti"
  } as const;

  return (
    <section class="page-panel" data-page="policy" aria-label={props.tx("policy.page")}>
      <main class="page-content">
        <section class="card preview-card">
          <SectionHeading title={props.tx("policy.preview")} />
          <div class="summary-line">
            <span class="summary-key">{props.tx("policy.summary.policy")}</span>
            <strong>{label(props.policyOptions.find((option) => option.value === props.model.settings().policy) ?? { value: "", labelKey: "" })}</strong>
            <span class="summary-key">{props.tx("policy.summary.format")}</span>
            <strong>{rateLabel(selectedRate(props.model.settings()))} · {label(props.bitDepthOptions.find((option) => option.value === props.model.settings().bitDepth) ?? { value: "", labelKey: "" })}</strong>
          </div>
          <div class="tag-row">
            <Show when={props.model.settings().drc}><span class="tag">DRC</span></Show>
            <Show when={props.model.settings().forceUsbv2}><span class="tag">USBv2</span></Show>
            <Show when={props.model.settings().forceBluetoothQti}><span class="tag">Bluetooth QTI</span></Show>
          </div>
          <div class="preview-actions">
            <button class="secondary-button" onClick={() => void props.model.reset()} disabled={props.model.busy()}>{props.model.activeOperation() === "policy.reset" ? props.tx("common.working") : props.tx("common.reset")}</button>
            <button class="primary-button" onClick={() => void props.model.apply()} disabled={props.model.busy()}>{props.model.activeOperation() === "policy.apply" ? props.tx("common.working") : props.tx("common.apply")}</button>
          </div>
        </section>

        <section class="grid two-col">
          <article class="card section-card">
            <SectionHeading title={props.tx("policy.section")} />
            <div class="field-label-row"><label class="field-label" for="policy">{props.tx("policy.template")}</label><button type="button" class="inline-link" onClick={props.onOpenHelp}>ⓘ {props.tx("policy.guide")}</button></div>
            <SelectField id="policy" title={props.tx("policy.template.select")} value={props.model.settings().policy} options={policySelectOptions()} language={props.language} onChange={(value) => props.model.update("policy", value)} />
          </article>

          <article class="card section-card">
            <SectionHeading title={props.tx("policy.format")} />
            <label class="field-label" for="rate">{props.tx("policy.rate")}</label>
            <SelectField id="rate" title={props.tx("policy.rate.select")} value={props.model.settings().rate} options={rateSelect()} language={props.language} onChange={props.model.changeRate} />
            <Show when={props.model.settings().rate === "custom"}><input class="number-input spaced-input page-swipe-input" inputmode="numeric" type="number" min={props.sampleRateLimit.min} max={props.sampleRateLimit.max} step={props.sampleRateLimit.step} value={props.model.settings().customRate} onInput={(event) => props.model.update("customRate", event.currentTarget.value)} placeholder={props.tx("policy.rate.customPlaceholder")} /></Show>
            <label class="field-label" for="bits">{props.tx("policy.bitDepth")}</label>
            <SelectField id="bits" title={props.tx("policy.bitDepth.select")} value={props.model.settings().bitDepth} options={bitSelect()} language={props.language} onChange={(value) => props.model.update("bitDepth", value)} />
          </article>
        </section>

        <section class="card section-card">
          <SectionHeading title={props.tx("policy.switches")} />
          <div class="switch-grid">
            <For each={props.switchOptions}>{(option) => {
              const key = switchBindings[option.value as keyof typeof switchBindings];
              return key
                ? <ToggleRow label={props.tx(option.labelKey)} description={option.descriptionKey ? props.tx(option.descriptionKey) : ""} checked={props.model.settings()[key]} onChange={(value) => props.model.update(key, value)} />
                : null;
            }}</For>
          </div>
        </section>

        <section class="status-strip card">
          <div><span class="label">audioserver</span><strong>{props.model.status().system.audioserverPid || props.tx("policy.status.notDetected")}</strong></div>
          <div><span class="label">{props.tx("policy.status.scriptVersion")}</span><strong>{props.model.status().system.scriptVersion || "—"}</strong></div>
          <div><span class="label">{props.tx("policy.status.sampleRate")}</span><strong>{props.model.status().policy.rate ? rateLabel(selectedRate(props.model.status().policy)) : "—"}</strong></div>
          <div><span class="label">Bluetooth A2DP</span><strong>{a2dpLabel()}</strong></div>
        </section>
        <Show when={props.model.status().system.stateDegraded}>
          <div class="notice danger-notice status-health"><strong>{props.tx("policy.status.degraded")}</strong><span>{props.model.status().system.stateDegradedReason || props.tx("policy.status.degradedUnknown")}</span></div>
        </Show>
      </main>
    </section>
  );
}
