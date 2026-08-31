import type { ExecResult } from "../domain/models";

export type ControllerOperationKind = "query" | "mutation";
export type ControllerOperationResult = "success" | "failed" | "timed_out" | "not_started";
export type ControllerOperationState = "not_started" | "applied" | "partially_applied" | "possibly_applied";

export type ControllerOperationMetadata = {
  kind?: ControllerOperationKind;
  result?: ControllerOperationResult;
  applied?: boolean;
  state?: ControllerOperationState;
  statePersist?: string;
  postCheck?: string;
  stateDegraded?: boolean;
  stateDegradedReason?: string;
};

export type ControllerStatus = {
  controller_version?: string;
  script_version?: string;
  module_dir?: string;
  policy?: string;
  sample_rate?: string;
  bit_depth?: string;
  drc?: string;
  force_usbv2?: string;
  force_bluetooth_qti?: string;
  bluetooth_a2dp_connected?: string;
  bluetooth_a2dp_state?: string;
  amzm?: string;
  test?: string;
  test_template?: string;
  audioserver_pid?: string;
  self_ns?: string;
  audio_ns?: string;
  namespace_ok?: string;
  last_action?: string;
  last_route?: string;
  last_exit?: string;
  last_time?: string;
  state_degraded?: string;
  state_degraded_reason?: string;
  /** Mutation envelope fields; present on immediate command results. */
  post_check?: string;
  state_persist?: string;
  state_migration?: string;
  state_recovery_reason?: string;
  last_command_log?: string;
  last_timed_out?: string;
  last_stdout_truncated?: string;
  last_stderr_truncated?: string;
  last_upstream_status?: string;
  last_post_check?: string;
  last_state_persist?: string;
  operation_kind?: string;
  operation_result?: string;
  operation_applied?: string;
  last_operation_applied?: string;
  operation_state?: string;
  last_operation_kind?: string;
  last_operation_result?: string;
  last_operation_state?: string;
  last_audit_status?: string;
  templates?: string[];
  policy_configured?: string;
  bluetooth_hal?: string;
  bluetooth_hal_configured?: string;
  resampler_preset?: string;
  resampler_configured?: string;
  resampler_bypass?: string;
  resampler_cheat?: string;
  resampler_stop_band?: string;
  resampler_half_length?: string;
  resampler_percent?: string;
  usb_period?: string;
  usb_period_configured?: string;
  diagnostic?: string;
  diagnostic_all?: string;
  io_scheduler?: string;
  io_tone?: string;
  wifi_no_restart?: string;
  auto_reapply?: string;
  [key: `jitter_${string}`]: string | string[] | undefined;
};

export type ControllerStatusResponse = {
  result: ExecResult;
  status: ControllerStatus;
};

export function parseControllerStatus(output: string): ControllerStatus {
  const result: ControllerStatus = {};
  const templates: string[] = [];
  let inTemplates = false;

  for (const line of output.split(/\r?\n/)) {
    if (line === "templates_begin") {
      inTemplates = true;
      continue;
    }
    if (line === "templates_end") {
      inTemplates = false;
      continue;
    }

    const separator = line.indexOf("=");
    if (separator < 1) continue;
    const key = line.slice(0, separator);
    const value = line.slice(separator + 1);
    if (inTemplates && key === "template") templates.push(value);
    else (result as Record<string, unknown>)[key] = value;
  }

  result.templates = templates;
  return result;
}

function operationKind(value: string | undefined): ControllerOperationKind | undefined {
  return value === "query" || value === "mutation" ? value : undefined;
}

function operationResult(value: string | undefined): ControllerOperationResult | undefined {
  return value === "success" || value === "failed" || value === "timed_out" || value === "not_started"
    ? value
    : undefined;
}

function operationState(value: string | undefined): ControllerOperationState | undefined {
  return value === "not_started" || value === "applied" || value === "partially_applied" || value === "possibly_applied"
    ? value
    : undefined;
}

/**
 * Parses the Controller result envelope independently from command output.
 * Queries intentionally have no mutation state; legacy Controllers may omit
 * the new kind/result fields and are represented by undefined values.
 */
export function parseControllerOperation(output: string): ControllerOperationMetadata {
  const fields = parseControllerStatus(output);
  return {
    kind: operationKind(fields.operation_kind),
    result: operationResult(fields.operation_result),
    applied: fields.operation_applied === undefined
      ? undefined
      : fields.operation_applied === "1",
    state: operationState(fields.operation_state),
    statePersist: fields.state_persist,
    postCheck: fields.post_check,
    stateDegraded: fields.state_degraded === undefined
      ? undefined
      : fields.state_degraded === "1",
    stateDegradedReason: fields.state_degraded_reason
  };
}

export function outputText(result: ExecResult): string {
  return `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
}

const controllerEnvelopeKeys = new Set([
  "controller_upstream_started",
  "controller_action", "operation_kind", "operation_result", "operation_applied",
  "operation_state", "controller_route", "controller_exit", "upstream_status",
  "upstream_exit", "post_check", "state_persist", "audit_status", "timed_out",
  "stdout_truncated", "stderr_truncated", "command_log", "script_summary",
  "batch_total_timeout_seconds", "batch_step", "batch_step_timeout_seconds",
  "batch_step_started", "batch_step_timed_out", "batch_step_exit",
  "batch_step_skipped", "batch_step_start_failed", "batch_restart",
  "batch_restart_timeout_seconds", "batch_restart_started", "batch_restart_timed_out",
  "batch_restart_exit", "batch_restart_skipped", "batch_restart_start_failed",
  "batch_timeout_scope", "batch_total_timed_out"
]);

function isControllerEnvelopeLine(line: string): boolean {
  const separator = line.indexOf("=");
  if (separator < 1) return line === "controller_upstream_started=1";
  const key = line.slice(0, separator);
  if (controllerEnvelopeKeys.has(key)) return true;
  return /^batch_(?:step|restart)_/.test(key);
}

/**
 * Returns only diagnostic payload. Controller execution metadata is printed
 * on the same stdout/stderr streams for CLI consumers, but must not appear in
 * the Diagnostics card as if it were AudioFlinger output.
 */
export function diagnosticText(result: ExecResult): string {
  const clean = (value: string) => value
    .split(/\r?\n/)
    .filter((line) => !isControllerEnvelopeLine(line))
    .filter((line) => !line.startsWith("namespace verified:"))
    .join("\n")
    .trim();
  const stdout = clean(result.stdout);
  const stderr = clean(result.stderr);
  return `${stdout}${stderr ? `${stdout ? "\n" : ""}[stderr]\n${stderr}` : ""}`.trim();
}
