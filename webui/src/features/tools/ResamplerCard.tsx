import { Show } from "solid-js";
import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import type { ToolAction } from "../../domain/options";
import { SectionHeading } from "../../shared/components/SectionHeading";
import { ToggleRow } from "../../shared/components/ToggleRow";
import type { Translator } from "../../shared/types";
import type { NumericRange, SchemaOption } from "../../platform/controller-schema";
import type { SchemaGroup } from "../../application/createSchemaModel";

export function ResamplerCard(props: {
  preset: string;
  bypass: string;
  cheat: boolean;
  stopBand: string;
  halfLength: string;
  percent: string;
  busy: boolean;
  canApply: boolean;
  canReset: boolean;
  toolAction?: ToolAction;
  language: Language;
  tx: Translator;
  presetGroups: ReadonlyArray<SchemaGroup>;
  bypassOptions: ReadonlyArray<SchemaOption>;
  limits: { stopBand: NumericRange; halfLength: NumericRange; cutoffPercent: NumericRange; cheatPercent: NumericRange };
  onPresetChange: (value: string) => void;
  onBypassChange: (value: string) => void;
  onCheatChange: (value: boolean) => void;
  onStopBandInput: (value: string) => void;
  onStopBandBlur: () => void;
  onHalfLengthInput: (value: string) => void;
  onHalfLengthBlur: () => void;
  onPercentInput: (value: string) => void;
  onPercentBlur: () => void;
  onReset: () => void;
  onApply: () => void;
}) {
  const optionLabel = (option: SchemaOption) => {
    const label = props.tx(option.labelKey);
    if (!option.descriptionKey) return label;
    const description = props.tx(option.descriptionKey);
    return description === option.descriptionKey ? label : `${label}\n${description}`;
  };
  return <article class="card tool-panel">
    <SectionHeading title={props.tx("tools.resampler.title")} />
    <p class="field-help">{props.tx("tools.resampler.description")}</p>
    <SelectField title={props.tx("tools.resampler.select")} value={props.preset} options={[]} groups={props.presetGroups.map(({ labelKey, options }) => ({ label: props.tx(labelKey), options: options.map((option) => [option.value, optionLabel(option)] as const) }))} language={props.language} disabled={props.busy || !props.canApply} onChange={props.onPresetChange} />
    <Show when={props.preset === "custom"}><div class="custom-resampler">
      <label class="field-label">{props.tx("tools.resampler.activationRate")}</label><SelectField title={props.tx("tools.resampler.activationRate.select")} value={props.bypass} options={props.bypassOptions.map((option) => [option.value, props.tx(option.labelKey)] as const)} language={props.language} disabled={props.busy || !props.canApply} onChange={props.onBypassChange} />
      <ToggleRow label={props.tx("resampler.mode.cheat.label")} description={props.tx("tools.resampler.cheatDescription")} checked={props.cheat} disabled={props.busy || !props.canApply} onChange={props.onCheatChange} />
      <label class="field-label">{props.tx("tools.resampler.stopBand")} (dB)<input class="number-input page-swipe-input" aria-label={`${props.tx("tools.resampler.stopBand")} (dB)`} inputmode="numeric" type="number" min={props.limits.stopBand.min} max={props.limits.stopBand.max} step={props.limits.stopBand.step} value={props.stopBand} onInput={(event) => props.onStopBandInput(event.currentTarget.value)} onBlur={props.onStopBandBlur} disabled={props.busy || !props.canApply} /></label>
      <label class="field-label">{props.tx("tools.resampler.halfLength")}<input class="number-input page-swipe-input" aria-label={props.tx("tools.resampler.halfLength")} inputmode="numeric" type="number" min={props.limits.halfLength.min} max={props.limits.halfLength.max} step={props.limits.halfLength.step} value={props.halfLength} onInput={(event) => props.onHalfLengthInput(event.currentTarget.value)} onBlur={props.onHalfLengthBlur} disabled={props.busy || !props.canApply} /></label>
      <label class="field-label">{props.cheat ? props.tx("tools.resampler.cheatPercent") : props.tx("tools.resampler.cutoffPercent")} (%)<input class="number-input page-swipe-input" aria-label={`${props.cheat ? props.tx("tools.resampler.cheatPercent") : props.tx("tools.resampler.cutoffPercent")} (%)`} inputmode="numeric" type="number" min={(props.cheat ? props.limits.cheatPercent : props.limits.cutoffPercent).min} max={(props.cheat ? props.limits.cheatPercent : props.limits.cutoffPercent).max} step={(props.cheat ? props.limits.cheatPercent : props.limits.cutoffPercent).step} value={props.percent} onInput={(event) => props.onPercentInput(event.currentTarget.value)} onBlur={props.onPercentBlur} disabled={props.busy || !props.canApply} /></label>
    </div></Show>
    <div class="inline-actions"><button class="secondary-button" disabled={props.busy || !props.canReset} onClick={props.onReset}>{props.toolAction === "resampler-reset" ? props.tx("common.working") : props.tx("common.reset")}</button><button class="primary-button" disabled={props.busy || !props.canApply} onClick={props.onApply}>{props.toolAction === "resampler" ? props.tx("common.working") : props.tx("common.apply")}</button></div>
  </article>;
}
