import { For, onMount, type Accessor } from "solid-js";
import type { Language } from "../i18n";
import { translate, translateRuntime } from "../i18n";
import {
  createA2dpGuard,
  createConfirmService,
  createNotificationCenter,
  createOperationCoordinator,
  createStatusCoordinator
} from "../application";
import { createSchemaModel } from "../application/createSchemaModel";
import { controllerClient } from "../platform/controller-client";
import { outputText, type ControllerStatus, type ControllerStatusResponse } from "../platform/controller-protocol";
import { deviceStatusFromControllerStatus, type DeviceStatus } from "../domain/device-status";
import {
  BrandIcon,
  LogIcon,
  PolicyIcon,
  RefreshIcon,
  SettingsIcon,
  ToolsIcon,
  TuneIcon
} from "../Icons";
import { PolicyPage } from "../features/policy/PolicyPage";
import { PolicyHelpDialog } from "../features/policy/PolicyHelpDialog";
import { createPolicyModel, type PolicyPageModel } from "../features/policy/createPolicyModel";
import { ToolsPage } from "../features/tools/ToolsPage";
import { createToolsModel, type ToolsPageModel } from "../features/tools/createToolsModel";
import { TuningPage } from "../features/tuning/TuningPage";
import { createTuningModel, type TuningPageModel } from "../features/tuning/createTuningModel";
import { SettingsPage } from "../features/settings/SettingsPage";
import { createSettingsModel } from "../features/settings/createSettingsModel";
import { ConfirmDialog } from "../shared/components/ConfirmDialog";
import { LogDialog } from "../shared/components/LogDialog";
import { ToastHost } from "../shared/components/ToastHost";
import { createOverlayManager, type OverlayManager } from "./createOverlayManager";
import { createPageNavigation, type PageId } from "./createPageNavigation";

declare const __WEBUI_VERSION__: string;

const WEBUI_VERSION = typeof __WEBUI_VERSION__ === "string" ? __WEBUI_VERSION__ : "dev";

const pageItems: Array<{ id: PageId; label: string; icon: typeof PolicyIcon }> = [
  { id: "policy", label: "nav.policy", icon: PolicyIcon },
  { id: "tools", label: "nav.tools", icon: ToolsIcon },
  { id: "tuning", label: "nav.tuning", icon: TuneIcon },
  { id: "settings", label: "nav.settings", icon: SettingsIcon }
];

export function App() {
  const operations = createOperationCoordinator();
  let language: Accessor<Language> = () => "en";
  let overlays: OverlayManager;

  const notifications = createNotificationCenter({
    translate: (message, params) => translateRuntime(language(), message, params)
  });
  const confirmations = createConfirmService({
    translate: (value) => translate(language(), value),
    translateMessage: (value) => translateRuntime(language(), value),
    onOpen: () => overlays.openConfirmHistory(),
    onClose: (_reason, external) => overlays.closeConfirmHistory(external)
  });
  const settingsModel = createSettingsModel({
    controller: controllerClient,
    operations,
    notifications,
    confirmations
  });
  language = settingsModel.language;
  const schemaModel = createSchemaModel(controllerClient);

  const navigation = createPageNavigation();
  overlays = createOverlayManager({
    activePage: navigation.activePage,
    confirmOpen: () => confirmations.request() !== undefined,
    onConfirmDismiss: () => confirmations.cancel(true)
  });

  let policyModel: PolicyPageModel;
  let toolsModel: ToolsPageModel;
  let tuningModel: TuningPageModel;
  let lastDeviceStatus: DeviceStatus = deviceStatusFromControllerStatus({});
  let lastControllerStatus: ControllerStatus = {};
  const statusCoordinator = createStatusCoordinator<DeviceStatus>({
    initialStatus: lastDeviceStatus,
    fetch: async (): Promise<{ result: ControllerStatusResponse["result"]; status: DeviceStatus }> => {
      const response = await controllerClient.status();
      lastControllerStatus = response.status;
      return {
        result: response.result,
        status: deviceStatusFromControllerStatus(response.status, lastDeviceStatus, schemaModel.schema())
      };
    },
    hydrators: {
      policy: (status) => policyModel.hydrate(status),
      tools: (status) => toolsModel.hydrate(status),
      tuning: (status) => tuningModel.hydrate(status)
    },
    hydrateAlways: settingsModel.hydrate,
    failureMessage: "app.status.readFailed",
    onSuccess: (status) => { lastDeviceStatus = status; }
  });
  const a2dp = createA2dpGuard({
    controller: controllerClient,
    confirmations,
    notifications,
    translate: settingsModel.tx,
    onStatus: (status: ControllerStatus) => {
      const next = deviceStatusFromControllerStatus(
        status,
        statusCoordinator.status(),
        schemaModel.schema()
      );
      lastDeviceStatus = next;
      statusCoordinator.replaceStatus(next);
    }
  });

  policyModel = createPolicyModel({
    controller: controllerClient,
    operations,
    notifications,
    confirmations,
    a2dp,
    statusCoordinator,
    sampleRateLimit: schemaModel.sampleRateLimit
  });
  toolsModel = createToolsModel({
    controller: controllerClient,
    operations,
    notifications,
    confirmations,
    a2dp,
    status: statusCoordinator,
    translate: settingsModel.tx,
    limits: {
      usbPeriod: schemaModel.usbPeriodLimit,
      resampler: schemaModel.resamplerLimits
    }
  });
  tuningModel = createTuningModel({
    controller: controllerClient,
    operations,
    notifications,
    confirmations,
    a2dp,
    statusCoordinator,
    language: settingsModel.language,
    translate: settingsModel.tx,
    highRiskFeatures: schemaModel.jitterHighRiskFeatures,
    audioRestartFeatures: schemaModel.jitterAudioRestartFeatures,
    resetFeatures: schemaModel.jitterResetFeatures,
    featureLabelKey: schemaModel.jitterFeatureLabelKey
  });

  async function refresh(showSuccess = true) {
    await operations.runExclusive("refresh", async () => {
      try {
        await statusCoordinator.refresh("all");
        if (showSuccess) notifications.success("app.status.updated");
      } catch (error) {
        notifications.error(error);
      }
    });
  }

  async function openLogs() {
    overlays.openOverlay("log");
    try {
      const result = await controllerClient.logs();
      if (result.code !== 0) {
        notifications.error(new Error(outputText(result) || translateRuntime(language(), "app.logs.readFailed", { code: result.code })));
        return;
      }
      notifications.setLog(outputText(result));
    } catch (error) {
      notifications.error(error);
    }
  }

  onMount(() => {
    void (async () => {
      // Status and schema are independent reads, so start them in parallel.
      // Startup deliberately uses read(), not refresh(), so no feature model
      // is hydrated with static assumptions while the schema is still loading.
      const statusRead = statusCoordinator.read().catch((error: unknown) => {
        notifications.error(error);
        return undefined;
      });
      const [status] = await Promise.all([statusRead, schemaModel.load()]);
      if (!status) return;
      const schema = schemaModel.schema();
      const finalStatus = schema && Object.keys(lastControllerStatus).length > 0
        ? deviceStatusFromControllerStatus(lastControllerStatus, undefined, schema)
        : status;
      lastDeviceStatus = finalStatus;
      statusCoordinator.replaceStatus(finalStatus);
      statusCoordinator.hydrate(finalStatus, "all");
    })();
  });

  return (
    <>
      <div class="app-shell">
        <header class="app-header">
          <div class="brand">
            <span class="brand-mark"><BrandIcon /></span>
            <span class="brand-copy"><strong>SampleRate Changer</strong><small>{settingsModel.tx("app.subtitle")}</small></span>
          </div>
          <button class="icon-button" aria-label={settingsModel.tx("app.refresh")} title={settingsModel.tx("app.refresh")} onClick={() => void refresh()} disabled={operations.busy()}><RefreshIcon /></button>
          <button class="icon-button" aria-label={settingsModel.tx("app.logs.open")} title={settingsModel.tx("app.logs.title")} onClick={() => void openLogs()}><LogIcon /></button>
        </header>

        <div class="page-viewport" ref={navigation.setViewport} onTouchStart={navigation.handleInputTouchStart} onTouchEnd={navigation.handleInputTouchEnd} onTouchCancel={navigation.handleInputTouchCancel}>
          <div class="page-track">
            <PolicyPage model={policyModel} language={settingsModel.language()} tx={settingsModel.tx} policyOptions={schemaModel.policyOptions()} rateOptions={schemaModel.rateOptions()} bitDepthOptions={schemaModel.bitDepthOptions()} switchOptions={schemaModel.policySwitchOptions()} sampleRateLimit={schemaModel.sampleRateLimit()} onOpenHelp={() => overlays.openOverlay("policy-help")} />
            <ToolsPage model={toolsModel} language={settingsModel.language()} tx={settingsModel.tx} schema={schemaModel} />
            <TuningPage model={tuningModel} language={settingsModel.language()} tx={settingsModel.tx} schema={schemaModel} />
            <SettingsPage model={settingsModel} version={WEBUI_VERSION} />
          </div>
        </div>

        <nav class="page-navigation" classList={{ dragging: navigation.pageDragging() }} style={{ "--page-progress": navigation.pageProgress() }} aria-label={settingsModel.tx("nav.main")}>
          <span class="page-nav-indicator" aria-hidden="true" />
          <For each={pageItems}>{(item, index) => {
            const PageIcon = item.icon;
            return <button class="page-nav-item" classList={{ active: navigation.activePage() === item.id }} aria-current={navigation.activeIndex() === index() ? "page" : undefined} onClick={() => navigation.activatePage(item.id)}><PageIcon /><span>{settingsModel.tx(item.label)}</span></button>;
          }}</For>
        </nav>
      </div>

      <ToastHost toasts={notifications.toasts()} />
      <ConfirmDialog request={confirmations.request()} tx={settingsModel.tx} onDismiss={() => confirmations.cancel()} onAccept={() => confirmations.accept()} />
      <PolicyHelpDialog open={overlays.policyHelpOpen()} policy={policyModel.settings().policy} policyOptions={schemaModel.policyOptions()} tx={settingsModel.tx} onClose={() => overlays.closeOverlay("policy-help")} />
      <LogDialog open={overlays.logOpen()} log={notifications.log()} tx={settingsModel.tx} onClose={() => overlays.closeOverlay("log")} />
    </>
  );
}
