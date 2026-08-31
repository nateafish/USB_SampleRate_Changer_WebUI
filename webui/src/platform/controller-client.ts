import type { ExecResult, PolicySettings } from "../domain/models";
import { selectedRate } from "../domain/policy";
import type {
  BluetoothHal,
  DiagnosticType,
  ResamplerSettings
} from "../domain/tools";
import type { JitterOperation } from "../domain/tuning";
import { executeRoot, type RootExecutor } from "./ksu-runtime";
import {
  parseControllerOperation,
  parseControllerStatus,
  type ControllerOperationMetadata,
  type ControllerStatusResponse
} from "./controller-protocol";
import { parseControllerSchema, type SchemaResponse } from "./controller-schema";

export const CONTROLLER_PATH = "/data/adb/modules/usb_samplerate_changer_webui/usbsrctl";

export type ControllerExecResult = ExecResult & {
  operation: ControllerOperationMetadata;
};

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\"'\"'")}'`;
}

function policyArguments(settings: PolicySettings): string[] {
  const args = [
    "--policy", settings.policy,
    "--sample-rate", selectedRate(settings),
    "--bit-depth", settings.bitDepth
  ];
  if (settings.drc) args.push("--drc");
  if (settings.forceUsbv2) args.push("--force-usbv2");
  if (settings.forceBluetoothQti) args.push("--force-bluetooth-qti");
  return args;
}

export class ControllerClient {
  constructor(
    private readonly execute: RootExecutor = executeRoot,
    private readonly controllerPath: string = CONTROLLER_PATH
  ) {}

  status(): Promise<ControllerStatusResponse> {
    return this.runController(["status"]).then((result) => ({
      result,
      status: parseControllerStatus(result.stdout)
    }));
  }

  schema(): Promise<SchemaResponse> {
    return this.runController(["schema", "--json"]).then((result) => ({
      result,
      schema: result.code === 0 ? parseControllerSchema(result.stdout) : undefined
    }));
  }

  logs(): Promise<ExecResult> {
    return this.runController(["logs"]);
  }

  apply(settings: PolicySettings): Promise<ControllerExecResult> {
    return this.runController(["apply", ...policyArguments(settings)]);
  }

  reset(): Promise<ControllerExecResult> {
    return this.runController(["reset"]);
  }

  extra(args: readonly string[]): Promise<ControllerExecResult> {
    return this.runController(["extra", ...args]);
  }

  setAutoReapply(enabled: boolean): Promise<ControllerExecResult> {
    return this.runController(["settings", "auto-reapply", enabled ? "enable" : "disable"]);
  }

  setBluetoothHal(hal: BluetoothHal): Promise<ControllerExecResult> {
    return this.extra(["bluetooth-hal", hal]);
  }

  applyResampler(settings: ResamplerSettings): Promise<ControllerExecResult> {
    if (settings.preset !== "custom") return this.extra(["resampler", settings.preset]);
    return this.extra([
      "resampler",
      "custom",
      settings.bypass,
      settings.cheat ? "cheat" : "cutoff",
      String(settings.stopBand),
      String(settings.halfLength),
      String(settings.percent)
    ]);
  }

  resetResampler(): Promise<ControllerExecResult> {
    return this.extra(["resampler", "reset"]);
  }

  setUsbPeriod(period: number): Promise<ControllerExecResult> {
    return this.extra(["usb-period", String(period)]);
  }

  resetUsbPeriod(): Promise<ControllerExecResult> {
    return this.extra(["usb-period", "reset"]);
  }

  diagnose(type: DiagnosticType, complete: boolean): Promise<ControllerExecResult> {
    return this.extra(["diagnose", type, ...(complete ? ["all"] : [])]);
  }

  setJitter(operation: JitterOperation): Promise<ControllerExecResult> {
    const args = ["jitter", operation.enabled ? "enable" : "disable", operation.feature];
    if (operation.feature === "io" && operation.enabled) {
      args.push(operation.ioScheduler ?? "*", operation.ioTone ?? "medium");
    }
    if (operation.feature === "wifi" && operation.enabled && operation.wifiNoRestart) {
      args.push("no-restart");
    }
    return this.extra(args);
  }

  resetJitter(): Promise<ControllerExecResult> {
    return this.extra(["jitter", "disable", "all"]);
  }

  openBluetoothSettings(): Promise<ExecResult> {
    return this.execute("am start -a android.settings.BLUETOOTH_SETTINGS");
  }

  private async runController(args: readonly string[]): Promise<ControllerExecResult> {
    const command = [this.controllerPath, ...args].map(shellQuote).join(" ");
    const result = await this.execute(command);
    return {
      ...result,
      operation: parseControllerOperation(result.stdout)
    };
  }
}

export const controllerClient = new ControllerClient();
