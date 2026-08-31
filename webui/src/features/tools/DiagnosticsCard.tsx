import { Show } from "solid-js";
import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import { SectionHeading } from "../../shared/components/SectionHeading";
import { ToggleRow } from "../../shared/components/ToggleRow";
import type { Translator } from "../../shared/types";
import type { SchemaOption } from "../../platform/controller-schema";

export function DiagnosticsCard(props: {
  diagnostic: string;
  completeOutput: boolean;
  completeOutputSupported: boolean;
  output: string;
  busy: boolean;
  canRun: boolean;
  language: Language;
  tx: Translator;
  options: ReadonlyArray<SchemaOption>;
  onDiagnosticChange: (value: string) => void;
  onCompleteOutputChange: (value: boolean) => void;
  onRun: () => void;
}) {
  return <article class="card tool-panel">
    <SectionHeading title={props.tx("tools.diagnostics.title")} />
    <p class="field-help">{props.tx("tools.diagnostics.description")}</p>
    <SelectField title={props.tx("tools.diagnostics.select")} value={props.diagnostic} options={props.options.map((option) => [option.value, props.tx(option.labelKey)] as const)} language={props.language} disabled={props.busy || !props.canRun} onChange={props.onDiagnosticChange} />
    <Show when={props.completeOutputSupported}><ToggleRow label={props.tx("diagnostics.complete_output.label")} description={props.tx("diagnostics.complete_output.description")} checked={props.completeOutput} disabled={props.busy || !props.canRun} onChange={props.onCompleteOutputChange} /></Show>
    <div class="inline-actions"><button class="primary-button" disabled={props.busy || !props.canRun} onClick={props.onRun}>{props.tx("tools.diagnostics.run")}</button></div>
    <pre class="diagnostic-output" aria-live="polite">{props.output === "tools.diagnostics.empty" ? props.tx(props.output) : props.output}</pre>
  </article>;
}
