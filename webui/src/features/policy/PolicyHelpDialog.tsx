import { For, Show } from "solid-js";
import { policyDetails } from "../../domain/options";
import type { SchemaOption } from "../../platform/controller-schema";
import type { Translator } from "../../shared/types";

export function PolicyHelpDialog(props: { open: boolean; policy: string; policyOptions: ReadonlyArray<SchemaOption>; tx: Translator; onClose: () => void }) {
  const fallbackSummary = (value: string) => `policy.option.${value}.description`;
  return (
    <Show when={props.open}>
      <div class="dialog-layer" data-no-page-drag role="presentation">
        <button class="dialog-backdrop" aria-label={props.tx("dialog.policyHelp.close")} onClick={props.onClose} />
        <section class="info-dialog" role="dialog" aria-modal="true" aria-labelledby="policy-help-title">
          <header><div><h2 id="policy-help-title">{props.tx("policy.help.title")}</h2><p>{props.tx("policy.help.description")}</p></div><button class="dialog-close" aria-label={props.tx("dialog.policyHelp.close")} onClick={props.onClose}>×</button></header>
          <div class="policy-guide-list">
            <For each={props.policyOptions}>{(option) => { const summaryKey = option.descriptionKey ?? fallbackSummary(option.value); return <article classList={{ current: option.value === props.policy }}><div><h3>{props.tx(option.labelKey)}</h3><Show when={option.value === props.policy}><span>{props.tx("common.current")}</span></Show></div><p class="policy-summary">{props.tx(summaryKey)}</p><p>{props.tx(policyDetails[option.value] ?? "")}</p></article>; }}</For>
          </div>
        </section>
      </div>
    </Show>
  );
}
