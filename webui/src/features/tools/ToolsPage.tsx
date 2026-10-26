import { Show } from "solid-js";
import type { Language } from "../../i18n";
import { ToolsIcon } from "../../Icons";
import type { Translator } from "../../shared/types";
import type { ToolsPageModel } from "./createToolsModel";
import { BluetoothHalCard } from "./BluetoothHalCard";
import { DiagnosticsCard } from "./DiagnosticsCard";
import { ResamplerCard } from "./ResamplerCard";
import { UsbPeriodCard } from "./UsbPeriodCard";
import type { SchemaModel } from "../../application/createSchemaModel";

export function ToolsPage(props: {
  model: ToolsPageModel;
  language: Language;
  tx: Translator;
  schema: SchemaModel;
}) {
  return (
    <section class="page-panel" aria-label={props.tx("tools.page")}>
      <main class="page-content">
        <div class="page-intro"><div class="intro-icon"><ToolsIcon /></div><div><h1>{props.tx("tools.title")}</h1></div></div>
        <section class="grid two-col extra-grid">
          <Show when={props.schema.toolAvailable("bluetoothHal")}><BluetoothHalCard value={props.model.bluetoothHal()} busy={props.model.busy()} canApply={props.schema.toolOperation("bluetoothHal", "set")} canReset={props.schema.toolOperation("bluetoothHal", "reset")} toolAction={props.model.toolAction()} language={props.language} tx={props.tx} options={props.schema.bluetoothHalOptions()} onChange={props.model.setBluetoothHal} onReset={() => void props.model.resetBluetoothHal()} onApply={() => void props.model.applyBluetoothHal()} /></Show>
          <Show when={props.schema.toolAvailable("resampler")}><ResamplerCard preset={props.model.resamplerPreset()} bypass={props.model.resamplerBypass()} cheat={props.model.resamplerCheat()} stopBand={props.model.resamplerStopBand()} halfLength={props.model.resamplerHalfLength()} percent={props.model.resamplerPercent()} busy={props.model.busy()} canApply={props.schema.toolOperation("resampler", props.model.resamplerPreset() === "custom" ? "set_custom" : "set_preset")} canReset={props.schema.toolOperation("resampler", "reset")} toolAction={props.model.toolAction()} language={props.language} tx={props.tx} presetGroups={props.schema.resamplerPresetGroups()} bypassOptions={props.schema.resamplerBypassOptions()} limits={props.schema.resamplerLimits()} onPresetChange={props.model.setResamplerPreset} onBypassChange={props.model.setResamplerBypass} onCheatChange={props.model.setResamplerCheat} onStopBandInput={props.model.setResamplerStopBand} onStopBandBlur={props.model.normalizeResamplerStopBand} onHalfLengthInput={props.model.setResamplerHalfLength} onHalfLengthBlur={props.model.normalizeResamplerHalfLength} onPercentInput={props.model.setResamplerPercent} onPercentBlur={props.model.normalizeResamplerPercent} onReset={() => void props.model.resetResampler()} onApply={() => void props.model.applyResampler()} /></Show>
          <Show when={props.schema.toolAvailable("usbPeriod")}><UsbPeriodCard value={props.model.usbPeriod()} normalizedValue={props.model.normalizedUsbPeriod()} busy={props.model.busy()} canApply={props.schema.toolOperation("usbPeriod", "set")} canReset={props.schema.toolOperation("usbPeriod", "reset")} toolAction={props.model.toolAction()} tx={props.tx} limit={props.schema.usbPeriodLimit()} onInput={props.model.setUsbPeriod} onBlur={props.model.normalizeUsbPeriod} onStep={props.model.stepUsbPeriod} onReset={() => void props.model.resetUsbPeriod()} onApply={() => void props.model.applyUsbPeriod()} /></Show>
          <Show when={props.schema.toolAvailable("diagnostics")}><DiagnosticsCard diagnostic={props.model.diagnostic()} completeOutput={props.model.diagnosticAll()} completeOutputSupported={props.schema.diagnosticsCompleteOutput()} output={props.model.diagnosticOutput()} busy={props.model.busy()} canRun={props.schema.toolOperation("diagnostics", "run")} language={props.language} tx={props.tx} options={props.schema.diagnosticOptions()} onDiagnosticChange={props.model.setDiagnostic} onCompleteOutputChange={props.model.setDiagnosticAll} onRun={() => void props.model.runDiagnostic()} /></Show>
        </section>
      </main>
    </section>
  );
}
