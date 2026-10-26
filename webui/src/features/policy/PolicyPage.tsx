import { For, Show } from "solid-js";
import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import type { PolicySettings } from "../../domain/models";
import { displayRate, selectedRate, groupPolicyOptions, policyOptionsForMode, policyWithDynamicCompatibility } from "../../domain/policy";
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
  const system = () => props.model.status().system;
  const isAidl = () => system().audioBackend === "aidl";
  const aidlLabel = () => system().aidlCoreVersion
    ? `AIDL v${system().aidlCoreVersion}`
    : "AIDL";
  const dynamicMode = () => props.model.settings().policy.endsWith("-dynamic");
  const visiblePolicyOptions = () => isAidl()
    ? props.policyOptions.filter(({ value }) => value !== "legacy" && !value.endsWith("-dynamic"))
    : policyOptionsForMode(props.policyOptions, props.model.settings().policy);
  const selectedPolicyLabel = () => label(props.policyOptions.find((option) => option.value === props.model.settings().policy) ?? { value: "", labelKey: "" });
  const policySelectGroups = () => groupPolicyOptions(visiblePolicyOptions()).map(({ labelKey, options }) => ({
    label: props.tx(labelKey),
    options: options.map((option) => [option.value,
      option.value === "legacy" || option.value === "legacy-dynamic"
        ? props.tx(`policy.option.${option.value}.listLabel`) : label(option)
    ] as const)
  }));
  const setDynamicCompatibility = (enabled: boolean) => {
    props.model.update(
      "policy",
      policyWithDynamicCompatibility(
        props.model.settings().policy,
        enabled,
        props.policyOptions.map(({ value }) => value)
      )
    );
    if (enabled && props.model.settings().forceBluetoothQti) {
      props.model.update("forceBluetoothQti", false);
    }
  };
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
          <div class="section-heading">
            <h2>{props.tx("policy.preview")}</h2>
            <Show when={isAidl()}>
              <span
                class="aidl-tip"
                title={props.tx("policy.aidl.tip")}
                aria-label={props.tx("policy.aidl.tip")}
              >{aidlLabel()}</span>
            </Show>
          </div>
          <div class="summary-line">
            <span class="summary-key">{props.tx("policy.summary.policy")}</span>
            <strong>{selectedPolicyLabel()}</strong>
          </div>
          <div class="summary-line">
            <span class="summary-key">{props.tx("policy.summary.format")}</span>
            <strong>{rateLabel(selectedRate(props.model.settings()))} · {label(props.bitDepthOptions.find((option) => option.value === props.model.settings().bitDepth) ?? { value: "", labelKey: "" })}</strong>
          </div>
          <div class="tag-row">
            <Show when={dynamicMode()}><span class="tag">{props.tx("policy.dynamic.status")}</span></Show>
            <Show when={props.model.settings().drc}><span class="tag">DRC</span></Show>
            <Show when={props.model.settings().forceUsbv2}><span class="tag">USBv2</span></Show>
            <Show when={props.model.settings().forceBluetoothQti}><span class="tag">Bluetooth QTI</span></Show>
          </div>
          <div class="preview-actions">
            <button class="secondary-button" onClick={() => void props.model.reset()} disabled={props.model.busy()}>{props.model.activeOperation() === "policy.reset" ? props.tx("common.working") : props.tx("common.reset")}</button>
            <button class="primary-button" onClick={() => void props.model.apply()} disabled={props.model.busy() || (isAidl() && !system().aidlApplySupported)}>{props.model.activeOperation() === "policy.apply" ? props.tx("common.working") : props.tx("common.apply")}</button>
          </div>
        </section>

        <section class="grid two-col">
          <article class="card section-card">
            <SectionHeading title={props.tx("policy.section")} />
            <div class="field-label-row"><label class="field-label" for="policy">{props.tx("policy.template")}</label><button type="button" class="inline-link" onClick={props.onOpenHelp}>ⓘ {props.tx("policy.guide")}</button></div>
            <SelectField id="policy" title={props.tx("policy.template.select")} value={props.model.settings().policy} options={[]} groups={policySelectGroups()} displayLabel={selectedPolicyLabel()} language={props.language} onChange={(value) => props.model.update("policy", value)} />
            <Show when={!isAidl()}><div class="policy-mode-toggle"><ToggleRow label={props.tx("policy.dynamic.label")} description={props.tx("policy.dynamic.description")} checked={dynamicMode()} onChange={setDynamicCompatibility} /></div></Show>
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

        <Show when={!isAidl()}><section class="card section-card">
          <SectionHeading title={props.tx("policy.switches")} />
          <div class="switch-grid">
            <For each={props.switchOptions}>{(option) => {
              const key = switchBindings[option.value as keyof typeof switchBindings];
              if (dynamicMode() && option.value === "force_bluetooth_qti") return null;
              return key
                ? <ToggleRow label={props.tx(option.labelKey)} description={option.descriptionKey ? props.tx(option.descriptionKey) : ""} checked={props.model.settings()[key]} onChange={(value) => props.model.update(key, value)} />
                : null;
            }}</For>
          </div>
        </section></Show>

        <Show when={isAidl()}>
          <details class="card aidl-card">
            <summary class="aidl-summary">
              <strong>{aidlLabel()}</strong>
              <span>{system().aidlConfigAdapter || "read-only"}</span>
              <span>{system().aidlConfigTopology || "—"}</span>
              <span>USB {system().aidlUsbReachableMixPortCount ?? 0}</span>
            </summary>
            <div class="aidl-detail-content">
              <div class="aidl-detail-row">
                <span>{props.tx("policy.aidl.source")}</span>
                <strong>{system().aidlConfigPath || system().audioConfigSource || "—"}</strong>
              </div>
              <div class="aidl-detail-row">
                <span>{props.tx("policy.aidl.behaviors")}</span>
                <strong class="aidl-port-behaviors">{system().aidlUsbPortBehaviors || "—"}</strong>
              </div>
              <div class="aidl-detail-row">
                <span>{props.tx("policy.aidl.dynamic")}</span>
                <strong>{system().aidlUsbDynamicPortCount ?? 0}</strong>
              </div>
              <Show when={system().aidlConfigError}>
                <p class="field-help">{system().aidlConfigError}</p>
              </Show>
            </div>
          </details>
        </Show>

        <section class="status-strip card">
          <div><span class="label">audioserver</span><strong>{props.model.status().system.audioserverPid || props.tx("policy.status.notDetected")}</strong></div>
          <div><span class="label">{props.tx("policy.status.scriptVersion")}</span><strong>{props.model.status().system.scriptVersion || "—"}</strong></div>
          <div><span class="label">{props.tx("policy.status.sampleRate")}</span><strong>{props.model.status().policy.rate ? rateLabel(selectedRate(props.model.status().policy)) : "—"}</strong></div>
          <div><span class="label">{props.tx("policy.status.bluetoothMedia")}</span><strong>{a2dpLabel()}</strong></div>
        </section>
        <Show when={props.model.status().system.stateDegraded}>
          <div class="notice danger-notice status-health"><strong>{props.tx("policy.status.degraded")}</strong><span>{props.model.status().system.stateDegradedReason || props.tx("policy.status.degradedUnknown")}</span></div>
        </Show>
      </main>
    </section>
  );
}
