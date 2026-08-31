import { For, Show } from "solid-js";
import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import { TuneIcon, WarningIcon } from "../../Icons";
import { ToggleRow } from "../../shared/components/ToggleRow";
import type { Translator } from "../../shared/types";
import type { TuningPageModel } from "./createTuningModel";
import type { SchemaModel } from "../../application/createSchemaModel";

export function TuningPage(props: {
  model: TuningPageModel;
  language: Language;
  tx: Translator;
  schema: SchemaModel;
}) {
  return (
    <section class="page-panel" aria-label={props.tx("tuning.page")}>
      <main class="page-content">
        <div class="page-intro danger-intro"><div class="intro-icon"><TuneIcon /></div><div><h1>{props.tx("tuning.title")}</h1></div></div>
        <Show when={props.schema.toolAvailable("jitter")}><section class="card section-card danger-card">
          <div class="notice danger-notice"><WarningIcon /><span><strong>{props.tx("tuning.risk.title")}</strong>{props.tx("tuning.risk.description")}</span></div>
          <div class="switch-grid jitter-grid"><For each={props.schema.jitterFeatures()}>{(feature) => <ToggleRow label={`${props.tx(feature.labelKey)}${props.model.dirty().includes(feature.value) ? props.tx("tuning.pending") : ""}`} description={feature.descriptionKey ? props.tx(feature.descriptionKey) : ""} checked={props.model.settings().jitter[feature.value]} disabled={props.model.busy() || !props.schema.toolOperation("jitter", "set")} onChange={(value) => props.model.updateFeature(feature.value, value)} />}</For></div>
          <Show when={props.model.settings().jitter.io && props.schema.jitterFeatureCapability("io", "io_parameters")}><div class="grid two-col io-options"><div><label class="field-label">{props.tx("tuning.io.scheduler")}</label><SelectField title={props.tx("tuning.io.scheduler.select")} value={props.model.settings().ioScheduler} options={props.schema.ioSchedulerOptions().map((option) => [option.value, props.tx(option.labelKey)] as const)} language={props.language} disabled={props.model.busy() || !props.schema.toolOperation("jitter", "set")} onChange={props.model.changeIoScheduler} /></div><div><label class="field-label">{props.tx("tuning.io.tone")}</label><SelectField title={props.tx("tuning.io.tone.select")} value={props.model.settings().ioTone} options={props.schema.ioToneOptions().map((option) => [option.value, props.tx(option.labelKey)] as const)} language={props.language} disabled={props.model.busy() || !props.schema.toolOperation("jitter", "set")} onChange={props.model.changeIoTone} /></div></div></Show>
          <Show when={props.model.settings().jitter.wifi && props.schema.jitterWifiNoRestart() && props.schema.jitterFeatureCapability("wifi", "wifi_no_restart")}><div class="wifi-option"><ToggleRow label={props.tx("jitter.wifi.no_restart.label")} description={props.tx("jitter.wifi.no_restart.description")} checked={props.model.settings().wifiNoRestart} disabled={props.model.busy() || !props.schema.toolOperation("jitter", "set")} onChange={props.model.changeWifiNoRestart} /></div></Show>
          <div class="inline-actions right tuning-actions"><button class="secondary-button" disabled={props.model.busy() || !props.schema.toolOperation("jitter", "reset")} onClick={() => void props.model.reset()}>{props.model.activeOperation() === "tuning.reset" ? props.tx("common.working") : props.tx("common.reset")}</button><button class="primary-button" disabled={props.model.busy() || !props.model.dirty().length || !props.schema.toolOperation("jitter", "set")} onClick={() => void props.model.apply()}>{props.model.activeOperation() === "tuning.apply" ? props.tx("common.working") : props.tx("common.apply")}</button></div>
        </section></Show>
      </main>
    </section>
  );
}
