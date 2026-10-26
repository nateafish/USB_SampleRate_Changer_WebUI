import { createSignal, type Accessor } from "solid-js";
import type {
  A2dpGuard,
  ConfirmService,
  NotificationCenter,
  OperationCoordinator,
  StatusCoordinator
} from "../../application";
import {
  buildJitterOperations,
  defaultTuningSettings,
  markJitterDirty,
  requiresHighRiskConfirmation,
  type TuningSettings
} from "../../domain/tuning";
import type { DeviceStatus } from "../../domain/device-status";
import type { JitterFeature } from "../../domain/options";
import type { Language } from "../../i18n";
import type { ControllerClient } from "../../platform/controller-client";
import { outputText } from "../../platform/controller-protocol";
import { controllerExecutionError } from "../../platform/controller-errors";
import type { Translator } from "../../shared/types";

type TuningController = Pick<ControllerClient, "setJitter" | "resetJitter">;
type TuningStatusCoordinator = StatusCoordinator<DeviceStatus>;

export type TuningPageModel = {
  settings: Accessor<TuningSettings>;
  dirty: Accessor<readonly JitterFeature[]>;
  busy: Accessor<boolean>;
  activeOperation: Accessor<string | undefined>;
  updateFeature: (feature: JitterFeature, value: boolean) => void;
  changeIoScheduler: (value: string) => void;
  changeIoTone: (value: string) => void;
  changeWifiNoRestart: (value: boolean) => void;
  hydrate: (status: DeviceStatus) => void;
  apply: () => Promise<void>;
  reset: () => Promise<void>;
};

export type CreateTuningModelOptions = {
  controller: TuningController;
  operations: OperationCoordinator;
  notifications: NotificationCenter;
  confirmations: ConfirmService;
  a2dp: A2dpGuard;
  statusCoordinator: TuningStatusCoordinator;
  language: Accessor<Language>;
  translate: Translator;
  highRiskFeatures?: Accessor<ReadonlySet<string>>;
  audioRestartFeatures?: Accessor<ReadonlySet<string>>;
  resetFeatures?: Accessor<ReadonlySet<string>>;
  featureLabelKey?: (feature: string) => string;
  initialSettings?: TuningSettings;
};

function commandError(
  feature: JitterFeature | "reset",
  result: Awaited<ReturnType<TuningController["setJitter"]>>
): Error {
  return controllerExecutionError(
    result,
    feature === "reset" ? "errors.resetFailed" : "errors.featureFailed",
    { feature, code: result.code }
  );
}

function highRiskMessage(
  operations: ReturnType<typeof buildJitterOperations>,
  translate: Translator,
  highRiskFeatures: ReadonlySet<string>,
  featureLabelKey: (feature: string) => string
): string {
  const riskActions = operations.flatMap(({ feature, enabled }) => {
    return enabled && highRiskFeatures.has(feature)
      ? [translate(featureLabelKey(feature))]
      : [];
  }).join(translate("tuning.risk.joiner"));

  return translate("tuning.risk.message", { actions: riskActions });
}

/**
 * Owns the editable jitter-reducer draft and its sequential mutation workflow.
 *
 * Each successful feature is removed from `dirty` immediately. A failing
 * feature and every operation after it stay dirty, making a retry resume from
 * precisely the unapplied part of the draft.
 */
export function createTuningModel(options: CreateTuningModelOptions): TuningPageModel {
  const [settings, setSettings] = createSignal<TuningSettings>(
    options.initialSettings ?? defaultTuningSettings()
  );
  const [dirty, setDirty] = createSignal<JitterFeature[]>([]);

  function markDirty(feature: JitterFeature) {
    setDirty((current) => markJitterDirty(current, feature));
  }

  function updateFeature(feature: JitterFeature, value: boolean) {
    setSettings((current) => ({
      ...current,
      jitter: { ...current.jitter, [feature]: value }
    }));
    markDirty(feature);
  }

  function changeIoScheduler(value: string) {
    setSettings((current) => ({ ...current, ioScheduler: value }));
    markDirty("io");
  }

  function changeIoTone(value: string) {
    setSettings((current) => ({ ...current, ioTone: value }));
    markDirty("io");
  }

  function changeWifiNoRestart(value: boolean) {
    setSettings((current) => ({ ...current, wifiNoRestart: value }));
    markDirty("wifi");
  }

  function hydrate(status: DeviceStatus) {
    setSettings(status.tuning);
    setDirty([]);
  }

  async function apply() {
    const pendingFeatures = [...dirty()];
    if (pendingFeatures.length === 0) {
      options.notifications.info("tuning.noChanges");
      return;
    }

    // Snapshot the entire draft before confirmation so the warning and the
    // commands it authorizes always describe the same settings.
    const draft = settings();
    const pendingOperations = buildJitterOperations(pendingFeatures, draft);
    const highRiskFeatures = options.highRiskFeatures?.() ?? new Set(["selinux", "thermal"]);
    const featureLabelKey = options.featureLabelKey ?? ((feature: string) => `jitter.${feature}.label`);
    if (requiresHighRiskConfirmation(pendingOperations, highRiskFeatures)) {
      const accepted = await options.confirmations.confirm({
        title: "tuning.risk.confirmTitle",
        message: highRiskMessage(
          pendingOperations,
          options.translate,
          highRiskFeatures,
          featureLabelKey
        ),
        confirmLabel: "tuning.risk.accept"
      });
      if (!accepted) return;
    }

    await options.operations.runExclusive("tuning.apply", async () => {
      const outputs: string[] = [];
      try {
        const audioRestartFeatures = options.audioRestartFeatures?.() ?? new Set(["effect"]);
        if (pendingFeatures.some((feature) => audioRestartFeatures.has(feature))) {
          options.notifications.info("tools.apply.progress");
        }

        for (const operation of pendingOperations) {
          const result = await options.controller.setJitter(operation);
          outputs.push(`[${operation.feature}]\n${outputText(result)}`.trim());

          if (options.a2dp.isRouteFailure(result)) {
            options.notifications.setLog(outputs.join("\n\n"));
            await options.a2dp.handleRouteFailure("tuning.title");
            return;
          }
          if (result.code !== 0) throw commandError(operation.feature, result);

          setDirty((current) => current.filter((item) => item !== operation.feature));
        }

        options.notifications.setLog(outputs.join("\n\n"));
        await options.statusCoordinator.refresh("tuning");
        options.notifications.success("tuning.apply.success");
      } catch (error) {
        options.notifications.setLog(outputs.join("\n\n"));
        options.notifications.error(error);
      }
    });
  }

  async function reset() {
    const accepted = await options.confirmations.confirm({
      title: "tuning.reset.title",
      message: "tuning.reset.message",
      confirmLabel: "common.confirmReset"
    });
    if (!accepted) return;

    const resetFeatures = options.resetFeatures?.()
      ?? new Set(Object.keys(settings().jitter));
    const retainedDirty = dirty().filter((feature) => !resetFeatures.has(feature));
    const retainedDraft = settings();

    await options.operations.runExclusive("tuning.reset", async () => {
      options.notifications.info("tuning.reset.progress");
      try {
        const result = await options.controller.resetJitter();
        const text = outputText(result);
        options.notifications.setLog(text || `exit=${result.code}`);

        if (options.a2dp.isRouteFailure(result)) {
          await options.a2dp.handleRouteFailure("tuning.reset.title");
          return;
        }
        if (result.code !== 0) throw commandError("reset", result);

        await options.statusCoordinator.refresh("tuning");
        if (retainedDirty.length > 0) {
          const retained = new Set(retainedDirty);
          setSettings((current) => ({
            ...current,
            jitter: {
              ...current.jitter,
              ...Object.fromEntries(retainedDirty.map((feature) => [feature, retainedDraft.jitter[feature]]))
            },
            ioScheduler: retained.has("io") ? retainedDraft.ioScheduler : current.ioScheduler,
            ioTone: retained.has("io") ? retainedDraft.ioTone : current.ioTone,
            wifiNoRestart: retained.has("wifi") ? retainedDraft.wifiNoRestart : current.wifiNoRestart
          }));
          setDirty(retainedDirty);
        }
        options.notifications.success("tuning.reset.success");
      } catch (error) {
        options.notifications.error(error);
      }
    });
  }

  return {
    settings,
    dirty,
    busy: options.operations.busy,
    activeOperation: options.operations.activeOperation,
    updateFeature,
    changeIoScheduler,
    changeIoTone,
    changeWifiNoRestart,
    hydrate,
    apply,
    reset
  };
}
