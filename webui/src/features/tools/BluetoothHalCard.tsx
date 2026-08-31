import SelectField from "../../SelectField";
import type { Language } from "../../i18n";
import { SectionHeading } from "../../shared/components/SectionHeading";
import type { ToolAction } from "../../domain/options";
import type { Translator } from "../../shared/types";
import type { SchemaOption } from "../../platform/controller-schema";

export function BluetoothHalCard(props: {
  value: string;
  busy: boolean;
  canApply: boolean;
  toolAction?: ToolAction;
  language: Language;
  tx: Translator;
  options: ReadonlyArray<SchemaOption>;
  onChange: (value: string) => void;
  onApply: () => void;
}) {
  return <article class="card tool-panel">
    <SectionHeading title={props.tx("tools.bluetoothHal.title")} />
    <p class="field-help">{props.tx("tools.bluetoothHal.description")}</p>
    <SelectField title={props.tx("tools.bluetoothHal.select")} value={props.value} options={props.options.map((option) => [option.value, props.tx(option.labelKey)] as const)} language={props.language} disabled={props.busy || !props.canApply} onChange={props.onChange} />
    <div class="inline-actions"><button class="primary-button" disabled={props.busy || !props.canApply} onClick={props.onApply}>{props.toolAction === "bluetooth-hal" ? props.tx("common.working") : props.tx("common.apply")}</button></div>
  </article>;
}
