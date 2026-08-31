import SelectField from "../../SelectField";
import { languageOptions } from "../../domain/options";
import { BrandIcon, SettingsIcon } from "../../Icons";
import { SectionHeading } from "../../shared/components/SectionHeading";
import { ToggleRow } from "../../shared/components/ToggleRow";
import type { SettingsPageModel } from "./createSettingsModel";

export function SettingsPage(props: {
  model: SettingsPageModel;
  version: string;
}) {
  return (
    <section class="page-panel" aria-label={props.model.tx("settings.page")}>
      <main class="page-content">
        <div class="page-intro"><div class="intro-icon"><SettingsIcon /></div><div><h1>{props.model.tx("settings.title")}</h1></div></div>
        <section class="card section-card">
          <SectionHeading title={props.model.tx("settings.language")} />
          <p class="field-help">{props.model.tx("settings.language.description")}</p>
          <div class="language-setting"><SelectField title={props.model.tx("settings.language.select")} value={props.model.pendingLanguage()} options={props.model.localizeOptions(languageOptions)} language={props.model.language()} onChange={(value) => props.model.setPendingLanguage(value as "zh-CN" | "en")} /><button type="button" class="primary-button" onClick={props.model.applyLanguage}>{props.model.tx("common.apply")}</button></div>
        </section>

        <section class="card section-card">
          <SectionHeading title={props.model.tx("settings.experimental")} />
          <ToggleRow label={props.model.tx("settings.reapply.label")} description={props.model.tx("settings.reapply.description")} checked={props.model.autoReapply()} onChange={(value) => void props.model.changeAutoReapply(value)} />
        </section>

        <section class="card section-card about-card">
          <SectionHeading title={props.model.tx("settings.about")} />
          <div class="about-brand"><span class="brand-mark"><BrandIcon /></span><div><h3>USB SampleRate Changer WebUI</h3><p>{props.model.tx("settings.about.subtitle")}</p></div></div>
          <div class="about-details"><div><span>{props.model.tx("settings.about.version")}</span><strong>{props.version}</strong></div></div>
          <p class="about-license">{props.model.tx("settings.about.license")}</p>
        </section>
      </main>
    </section>
  );
}
