import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount
} from "solid-js";
import { render } from "solid-js/web";
import EmblaCarousel, { type EmblaCarouselType } from "embla-carousel";
import "./styles.css";
import SelectField from "./SelectField";
import { type Language, translate, translateRuntime } from "./i18n";
import {
  BrandIcon,
  LogIcon,
  PolicyIcon,
  RefreshIcon,
  SettingsIcon,
  ToolsIcon,
  TuneIcon,
  WarningIcon
} from "./Icons";

declare const __WEBUI_VERSION__: string;

type ExecResult = { code: number; stdout: string; stderr: string };
type ToastItem = { id: string; message: string; tone?: "success" | "error" };
type ConfirmRequest = {
  title: string;
  message: string;
  confirmLabel: string;
  action: () => void;
  cancel?: () => void;
  showCancel?: boolean;
};

declare global {
  interface Window {
    __usbSrCallbacks?: Record<string, (code: number, stdout: string, stderr: string) => void>;
    ksu?: {
      exec?: (command: string, options?: string, callback?: string) => void | string | Promise<unknown>;
      spawn?: (command: string, args: string, options?: string, callback?: string) => void;
    };
  }
}

const CONTROLLER = "/data/adb/modules/usb_samplerate_changer_webui/usbsrctl";
const WEBUI_VERSION = typeof __WEBUI_VERSION__ === "string" ? __WEBUI_VERSION__ : "dev";

function detectLanguage(): Language {
  const locales = [
    ...(Array.isArray(navigator.languages) ? navigator.languages : []),
    navigator.language
  ].filter((locale): locale is string => typeof locale === "string" && locale.length > 0);
  return locales.some((locale) => /^zh(?:-|$)/i.test(locale)) ? "zh-CN" : "en";
}

const pages = ["policy", "tools", "tuning", "settings"] as const;
type PageId = typeof pages[number];
const pageItems: Array<{ id: PageId; label: string; icon: typeof PolicyIcon }> = [
  { id: "policy", label: "策略", icon: PolicyIcon },
  { id: "tools", label: "工具", icon: ToolsIcon },
  { id: "tuning", label: "调优", icon: TuneIcon },
  { id: "settings", label: "设置", icon: SettingsIcon }
];

const policyOptions = [
  ["auto", "自动检测", "根据设备 HAL 和 Bluetooth 能力选择模板"],
  ["offload", "硬件卸载", "USB 与蓝牙硬件卸载"],
  ["offload-hifi-playback", "USB HiFi Playback", "USB hifi_playback 硬件卸载"],
  ["offload-direct", "Direct PCM", "使用 direct_pcm / compressed_offload"],
  ["offload-safer", "较安全卸载", "保留蓝牙策略，使用较安全的 USB 卸载"],
  ["bypass", "绕过硬件卸载", "绕过 USB 与蓝牙硬件卸载"],
  ["bypass-safer", "较安全绕过", "绕过卸载但保留传统内部输出设置"],
  ["legacy", "Legacy A2DP", "旧版 Bluetooth A2DP HAL"],
  ["safe", "Safe", "传统兼容设置"],
  ["safest", "Safest", "最大兼容性设置"],
  ["safest-auto", "Safest Auto", "兼容性设置并自动探测 USB 上限"],
  ["usb", "USB Only", "只修改 USB 音频策略"]
] as const;

const policyDetails: Record<string, string> = {
  auto: "自动检查设备的音频策略格式、USB HAL 与 Bluetooth 实现，再选择匹配模板。适合首次使用和 ROM 更新后的重新探测；实际使用的 XML 可能随设备环境变化。",
  offload: "同时保留 USB 与 Bluetooth 的硬件卸载路径，让受支持的音频流尽量绕过 AudioFlinger 混音。适合卸载链路完整的设备；HAL 或厂商路由不兼容时可能出现无声。",
  "offload-hifi-playback": "使用 hifi_playback 输出路径进行 USB 硬件卸载。只适合明确提供该输出 profile 的 ROM；没有对应 profile 时通常无法建立 USB 输出。",
  "offload-direct": "优先使用 direct_pcm / compressed_offload 一类直接输出路径，减少系统混音介入。适合 Qualcomm 等提供 Direct PCM 的设备。",
  "offload-safer": "使用较保守的 USB 卸载配置，同时尽量保留原有 Bluetooth 与内部扬声器路由。适合标准 offload 会破坏其他输出设备时使用。",
  bypass: "绕过 USB 与 Bluetooth 的硬件卸载，让音频回到 AudioFlinger 混音和软件重采样路径。兼容性通常更高，但不再追求硬件直通。",
  "bypass-safer": "在绕过卸载的基础上保留更多传统内部输出与厂商路由定义。适合普通 bypass 导致蓝牙、扬声器或听筒异常的设备。",
  legacy: "使用旧版 A2DP / bluetooth_qti 风格策略。面向仍依赖传统 Bluetooth Audio HAL 的旧 ROM；现代 AIDL/HIDL 蓝牙栈通常不应优先选择。",
  safe: "采用保守的传统兼容配置，减少对厂商音频策略结构的改动。适合排查高级模板造成的路由问题。",
  safest: "进一步降低模板假设，只保留最通用的输出定义。适合恢复声音或确认问题是否来自厂商专有路径，功能和直通能力也最有限。",
  "safest-auto": "以最大兼容模板为基础，同时自动探测 USB 采样率上限。适合未知 DAC 或旧版策略格式，但探测结果仍受内核与 USB HAL 能力限制。",
  usb: "只修改 USB 音频相关策略，不主动改变 Bluetooth 和其他内部输出。适合仅需要外接 USB DAC、并希望最大限度降低系统其他路由影响的场景。"
};
const aidlPolicyOverrides: Record<string, readonly [string, string, string]> = {
  "offload-hifi-playback": [
    "厂商动态直通",
    "选择 USB 可达的动态输出 profile",
    "按动态 profile、路由可达性和端口行为选择厂商直通路径，不依赖 hifi_playback 或其他专机端口名。"
  ],
  usb: [
    "USB 动态端口",
    "只规划 USB 可达的动态输出 profile",
    "只选择能路由到 USB 输出且 profile 为动态的 mix port；不会把独立 USB accessory 模块误当作普通 USB DAC。"
  ]
};
const policySelectOptions = policyOptions.map(([value, label]) => [value, label] as const);

const rateOptions = [
  ["44100", "44.1 kHz"],
  ["48000", "48 kHz"],
  ["88200", "88.2 kHz"],
  ["96000", "96 kHz"],
  ["176400", "176.4 kHz"],
  ["192000", "192 kHz"],
  ["352800", "352.8 kHz"],
  ["384000", "384 kHz"],
  ["705600", "705.6 kHz"],
  ["768000", "768 kHz"]
] as const;

const bitOptions = [
  ["16", "16-bit PCM"],
  ["24", "24-bit packed PCM"],
  ["32", "32-bit PCM"],
  ["float", "32-bit float PCM"]
] as const;

const bluetoothHalOptions = [
  ["offload", "硬件卸载（推荐）"],
  ["aosp", "AOSP Bluetooth HAL"],
  ["legacy", "Legacy / bluetooth_qti HAL"],
  ["sysbta", "System Bluetooth Audio HAL"]
] as const;

const resamplerOptions = [
  ["159-480-92", "旧版默认\n159dB / 480 / 92%(cutoff)"],
  ["165-360-104", "低性能设备\n165dB / 360 / 104%(cheat)"],
  ["179-408-99", "Android 12+（默认）\n179dB / 408 / 99%(cheat)"],
  ["194-520-100", "1:1 Bit-perfect\n194dB / 520 / 100%(cutoff)"],
  ["ultra-hifi", "Ultra HiFi\n194dB / 520 / 98%(cheat)"],
  ["cheap-44", "目标音源 44.1kHz\n194dB / 520 / 92%(cutoff)"],
  ["cheap-44-low", "目标音源 44.1kHz 兼容模式\n194dB / 520 / 91%(cutoff)"],
  ["cheap-48", "目标音源 48kHz\n194dB / 520 / 84%(cutoff)"],
  ["cheap-48-low", "目标音源 48kHz 兼容模式\n194dB / 520 / 83%(cutoff)"],
  ["cheap-96", "目标音源 96kHz\n194dB / 520 / 42%(cutoff)"],
  ["mock-dac-a", "AK4491EQ 陡峭滚降\n150dB / 80 / 109%(cheat)"],
  ["mock-dac-b", "ES9039PRO 快速滚降\n120dB / 80 / 97%(cutoff)"],
  ["mock-dac-c", "更好的SoX HQ线性相位采样\n100dB / 80 / 104%(cheat)"],
  ["mock-mastering", "iZotope无混叠采样\n159dB / 240 / 99%(cheat)"]
] as const;

const diagnosticOptions = [
  ["audio", "Audio policy / AudioFlinger"],
  ["bluetooth", "Bluetooth codec 与 A2DP"],
  ["config", "音频配置文件探测"],
  ["alsa", "ALSA hw_params / DAC profile"]
] as const;
const languageOptions = [["zh-CN", "中文"], ["en", "English"]] as const;

const rateSelectOptions = [...rateOptions, ["custom", "自定义整数（44100–768000 Hz）"]] as const;
const resamplerCustomOption = ["custom", "自定义完整参数"] as const;
const resamplerPresetGroups = [
  { label: "常规", options: [...resamplerOptions.slice(0, 5), resamplerCustomOption] },
  { label: "非线性DAC/蓝牙耳机", options: resamplerOptions.slice(5, 10) },
  { label: "模拟滤波", options: resamplerOptions.slice(10) }
] as const;
const resamplerBypassOptions = [["none", "44.1 kHz 起"], ["48", "48 kHz 起"], ["96", "96 kHz 起"]] as const;
const ioSchedulerOptions = ["*", "none", "noop", "deadline", "mq-deadline", "cfq", "bfq", "kyber"].map((value) => [value, value === "*" ? "自动选择" : value] as const);
const ioToneOptions = ["light", "m-light", "medium", "boost", "exp"].map((value) => [value, value] as const);

const jitterFeatures = [
  ["selinux", "允许 SELinux Permissive", "打开后允许系统进入 Permissive；关闭并应用则恢复 Enforcing。安全风险高。"],
  ["thermal", "停用温控", "打开后停止或弱化温控服务；关闭并应用则尝试恢复系统温控。"],
  ["doze", "停用 Doze", "打开后停用设备空闲省电机制，减少后台唤醒带来的调度变化。"],
  ["governor", "锁定高性能调频", "打开后将支持的 CPU/GPU 调频策略设为高性能，减少频率切换。"],
  ["camera", "停用相机服务", "打开后停止相机后台服务；需要使用相机时请关闭并应用。"],
  ["logd", "停用日志服务", "打开后停止 logd 与部分追踪服务，减少持续日志写入。"],
  ["io", "优化 I/O 调度", "打开后调整调度器、预读和队列参数，降低存储访问干扰。"],
  ["vm", "优化虚拟内存", "打开后调整 swappiness 与脏页回写策略，减少突发写入。"],
  ["wifi", "停用 Wi-Fi 省电", "打开后停用 Wi-Fi 挂起优化和自适应连接；部分修改可能跨重启保留。"],
  ["battery", "停用自适应电池", "打开后停用自适应电池与自适应充电管理，减少后台干预。"],
  ["effect", "停用系统音效", "打开后绕过系统音效框架及相关服务，减少额外音频后处理。"]
] as const;

type JitterFeature = typeof jitterFeatures[number][0];
type JitterValues = Record<JitterFeature, boolean>;
type RefreshScope = "all" | "policy" | "tools" | "tuning";
type ToolAction = "bluetooth-hal" | "resampler" | "resampler-reset" | "usb-period" | "usb-period-reset" | "jitter" | "jitter-reset";

const defaultJitterValues = (): JitterValues => Object.fromEntries(
  jitterFeatures.map(([key]) => [key, false])
) as JitterValues;

type Settings = {
  policy: string;
  rate: string;
  customRate: string;
  bitDepth: string;
  drc: boolean;
  forceUsbv2: boolean;
  forceBluetoothQti: boolean;
};

type Status = {
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
  audio_backend?: string;
  audio_config_source?: string;
  aidl_core_version?: string;
  aidl_core_versions?: string;
  aidl_version_supported?: string;
  aidl_module_count?: string;
  aidl_usb_output_device_count?: string;
  aidl_usb_reachable_mix_port_count?: string;
  aidl_usb_port_behaviors?: string;
  aidl_usb_port_summary?: string;
  aidl_config_adapter?: string;
  aidl_config_path?: string;
  aidl_config_topology?: string;
  aidl_config_error?: string;
  aidl_preview_supported?: string;
  aidl_apply_supported?: string;
  [key: `aidl_supported_rates_${string}`]: string | undefined;
  [key: `jitter_${string}`]: string | string[] | undefined;
};

const defaultSettings = (): Settings => ({
  policy: "auto",
  rate: "44100",
  customRate: "44100",
  bitDepth: "32",
  drc: false,
  forceUsbv2: false,
  forceBluetoothQti: false
});

let callbackSequence = 0;
let spawnSequence = 0;

type SpawnStream = {
  on: (event: "data", callback: (data: string) => void) => void;
  emit: (event: "data", data: string) => void;
};

type SpawnChild = {
  stdout: SpawnStream;
  stderr: SpawnStream;
  on: (event: "exit" | "error", callback: (value: number | Error) => void) => void;
  emit: (event: "exit" | "error", value: number | Error) => void;
};

function createSpawnStream(): SpawnStream {
  const listeners: Array<(data: string) => void> = [];
  return {
    on(event, callback) {
      if (event === "data") listeners.push(callback);
    },
    emit(event, data) {
      if (event === "data") listeners.forEach((callback) => callback(data));
    }
  };
}

function createSpawnChild(): SpawnChild {
  const listeners: Record<"exit" | "error", Array<(value: number | Error) => void>> = {
    exit: [],
    error: []
  };
  return {
    stdout: createSpawnStream(),
    stderr: createSpawnStream(),
    on(event, callback) {
      listeners[event].push(callback);
    },
    emit(event, value) {
      listeners[event].forEach((callback) => callback(value));
    }
  };
}

function rootSpawn(command: string): Promise<ExecResult> {
  return new Promise((resolve, reject) => {
    const ksu = window.ksu;
    if (!ksu || typeof ksu.spawn !== "function") {
      reject(new Error("KernelSU/APatch WebUI spawn 接口不可用"));
      return;
    }

    const id = `spawn${Date.now().toString(36)}_${(++spawnSequence).toString(36)}`;
    const callbackRef = `__usbSrSpawn_${id}`;
    const child = createSpawnChild();
    const callbacks = window as unknown as Record<string, unknown>;
    const stdout: string[] = [];
    const stderr: string[] = [];
    let settled = false;

    const cleanup = () => {
      delete callbacks[callbackRef];
      window.clearTimeout(timeout);
    };
    const fail = (error: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error instanceof Error ? error : new Error(String(error)));
    };

    child.stdout.on("data", (data) => stdout.push(String(data ?? "")));
    child.stderr.on("data", (data) => stderr.push(String(data ?? "")));
    child.on("exit", (code) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve({
        code: Number(code),
        stdout: stdout.join("\n"),
        stderr: stderr.join("\n")
      });
    });
    child.on("error", fail);
    callbacks[callbackRef] = child;

    const timeout = window.setTimeout(() => {
      fail(new Error("root 命令执行超时"));
    }, 120000);

    try {
      // KernelSU and APatch both expect JSON-encoded args/options here. Keep
      // the existing shell-quoted command so this remains compatible with the
      // controller's current command construction and the exec fallback.
      ksu.spawn(command, "[]", "{}", callbackRef);
    } catch (error) {
      fail(error);
    }
  });
}

function rootExec(command: string): Promise<ExecResult> {
  if (typeof window.ksu?.spawn === "function") return rootSpawn(command);
  return new Promise((resolve, reject) => {
    const ksu = window.ksu;
    if (!ksu || typeof ksu.exec !== "function") {
      reject(new Error("未检测到 KernelSU/APatch WebUI root 接口"));
      return;
    }
    const id = `cb${Date.now().toString(36)}_${(++callbackSequence).toString(36)}`;
    const callbacks: Record<string, (code: number, stdout: string, stderr: string) => void> =
      window.__usbSrCallbacks ?? (window.__usbSrCallbacks = Object.create(null));
    const callbackRef = `window.__usbSrCallbacks.${id}`;
    const timeout = window.setTimeout(() => {
      delete callbacks[id];
      reject(new Error("root 命令执行超时"));
    }, 120000);
    callbacks[id] = (code, stdout, stderr) => {
      window.clearTimeout(timeout);
      delete callbacks[id];
      resolve({ code: Number(code), stdout: String(stdout || ""), stderr: String(stderr || "") });
    };
    try {
      void ksu.exec(command, "{}", callbackRef);
    } catch (error) {
      window.clearTimeout(timeout);
      delete callbacks[id];
      reject(error);
    }
  });
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\"'\"'")}'`;
}

function parseStatus(output: string): Status {
  const result: Status = {};
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

function rateValue(settings: Settings): string {
  return settings.rate === "custom" ? settings.customRate : settings.rate;
}

function controllerArgs(settings: Settings): string {
  const args = [
    "--policy", settings.policy,
    "--sample-rate", rateValue(settings),
    "--bit-depth", settings.bitDepth
  ];
  if (settings.drc) args.push("--drc");
  if (settings.forceUsbv2) args.push("--force-usbv2");
  if (settings.forceBluetoothQti) args.push("--force-bluetooth-qti");
  return args.map(shellQuote).join(" ");
}

function displayRate(value: string): string {
  const known = rateOptions.find(([rate]) => rate === value);
  if (known) return known[1];
  const numeric = Number(value);
  return Number.isFinite(numeric) ? `${numeric.toLocaleString()} Hz` : "未设置";
}

function initialFromStatus(status: Status): Settings {
  const settings = defaultSettings();
  if (status.policy) settings.policy = status.policy;
  if (status.sample_rate) {
    const known = rateOptions.some(([rate]) => rate === status.sample_rate);
    settings.rate = known ? status.sample_rate : "custom";
    settings.customRate = status.sample_rate;
  }
  if (status.bit_depth) settings.bitDepth = status.bit_depth;
  if (status.audio_backend === "aidl" && settings.policy === "legacy") settings.policy = "auto";
  if (status.audio_backend === "aidl") {
    const supported = status[`aidl_supported_rates_${settings.policy.replaceAll("-", "_")}`]
      ?.split(",")
      .filter(Boolean) ?? [];
    const selected = settings.rate === "custom" ? settings.customRate : settings.rate;
    if (supported.length && !supported.includes(selected)) {
      settings.rate = supported[0];
      settings.customRate = supported[0];
    }
  }
  settings.drc = status.drc === "1";
  settings.forceUsbv2 = status.force_usbv2 === "1";
  settings.forceBluetoothQti = status.force_bluetooth_qti === "1";
  return settings;
}

function App() {
  const [settings, setSettings] = createSignal<Settings>(defaultSettings());
  const [status, setStatus] = createSignal<Status>({});
  const [busy, setBusy] = createSignal(false);
  const [toolAction, setToolAction] = createSignal<ToolAction>();
  const [toasts, setToasts] = createSignal<ToastItem[]>([]);
  const [log, setLog] = createSignal("");
  const [policyHelpOpen, setPolicyHelpOpen] = createSignal(false);
  const [bluetoothHal, setBluetoothHal] = createSignal("offload");
  const [resamplerPreset, setResamplerPreset] = createSignal("179-408-99");
  const [resamplerBypass, setResamplerBypass] = createSignal("none");
  const [resamplerCheat, setResamplerCheat] = createSignal(true);
  const [resamplerStopBand, setResamplerStopBand] = createSignal("179");
  const [resamplerHalfLength, setResamplerHalfLength] = createSignal("408");
  const [resamplerPercent, setResamplerPercent] = createSignal("99");
  const [usbPeriod, setUsbPeriod] = createSignal("2250");
  const [diagnostic, setDiagnostic] = createSignal("audio");
  const [diagnosticAll, setDiagnosticAll] = createSignal(false);
  const [diagnosticOutput, setDiagnosticOutput] = createSignal("尚未运行诊断。");
  const [jitterValues, setJitterValues] = createSignal<JitterValues>(defaultJitterValues());
  const [jitterDirty, setJitterDirty] = createSignal<JitterFeature[]>([]);
  const [ioScheduler, setIoScheduler] = createSignal("*");
  const [ioTone, setIoTone] = createSignal("medium");
  const [wifiNoRestart, setWifiNoRestart] = createSignal(false);
  const [autoReapply, setAutoReapply] = createSignal(false);
  const [activePage, setActivePage] = createSignal<PageId>("policy");
  const [pageProgress, setPageProgress] = createSignal(0);
  const [pageDragging, setPageDragging] = createSignal(false);
  const [logOpen, setLogOpen] = createSignal(false);
  const [confirmRequest, setConfirmRequest] = createSignal<ConfirmRequest>();
  const storedLanguage = window.localStorage.getItem("usbSrLanguage");
  const initialLanguage: Language = storedLanguage === "en"
    ? "en"
    : storedLanguage === "zh-CN"
      ? "zh-CN"
      : detectLanguage();
  const [language, setLanguage] = createSignal<Language>(initialLanguage);
  const [pendingLanguage, setPendingLanguage] = createSignal<Language>(language());
  const isAidl = createMemo(() => status().audio_backend === "aidl");

  createEffect(() => {
    document.documentElement.lang = language();
  });

  const tx = (value: string) => translate(language(), value);
  const localizeOptions = (options: ReadonlyArray<readonly [string, string]>) =>
    options.map(([value, label]) => [value, tx(label)] as const);
  const visiblePolicyOptions = () => policyOptions.filter(([value]) => !isAidl() || value !== "legacy");
  const policyLabel = (value: string, fallback: string) => isAidl()
    ? aidlPolicyOverrides[value]?.[0] ?? fallback
    : fallback;
  const policySummary = (value: string, fallback: string) => isAidl()
    ? aidlPolicyOverrides[value]?.[1] ?? fallback
    : fallback;
  const policyDescription = (value: string) => isAidl()
    ? aidlPolicyOverrides[value]?.[2] ?? policyDetails[value]
    : policyDetails[value];
  const localizePolicyOptions = () => visiblePolicyOptions()
    .map(([value, label]) => [value, tx(policyLabel(value, label))] as const);
  const availableRateOptions = () => {
    if (!isAidl()) return rateSelectOptions;
    const policyKey = settings().policy.replaceAll("-", "_");
    const supported = status()[`aidl_supported_rates_${policyKey}`]?.split(",").filter(Boolean);
    if (!supported?.length) return rateSelectOptions;
    return [
      ...rateOptions.filter(([rate]) => supported.includes(rate)),
      ["custom", "自定义整数（44100–768000 Hz）"] as const
    ];
  };

  let pageViewport: HTMLDivElement | undefined;
  let carousel: EmblaCarouselType | undefined;
  let inputSwipeStart: { x: number; y: number } | undefined;

  function handleInputTouchStart(event: TouchEvent) {
    const target = event.target;
    if (!(target instanceof Element) || !target.closest(".page-swipe-input")) return;
    const touch = event.touches[0];
    if (touch) inputSwipeStart = { x: touch.clientX, y: touch.clientY };
  }

  function handleInputTouchEnd(event: TouchEvent) {
    if (!inputSwipeStart) return;
    const touch = event.changedTouches[0];
    const start = inputSwipeStart;
    inputSwipeStart = undefined;
    if (!touch) return;
    const dx = touch.clientX - start.x;
    const dy = touch.clientY - start.y;
    if (Math.abs(dx) < 42 || Math.abs(dx) < Math.abs(dy) * 1.2) return;
    event.preventDefault();
    if (dx < 0) carousel?.scrollNext();
    else carousel?.scrollPrev();
  }

  const activeIndex = createMemo(() => pages.indexOf(activePage()));

  function syncCarousel() {
    if (!carousel) return;
    const progress = Math.max(0, Math.min(1, carousel.scrollProgress()));
    setPageProgress(progress * (pages.length - 1));
    const nextPage = pages[carousel.selectedScrollSnap()] ?? "policy";
    if (nextPage === activePage()) return;
    setActivePage(nextPage);
    window.history.replaceState({ ...window.history.state, usbSrPage: nextPage }, "");
  }

  function activatePage(page: PageId) {
    const index = pages.indexOf(page);
    if (!carousel) {
      setActivePage(page);
      setPageProgress(index);
      return;
    }
    carousel.scrollTo(index);
  }

  function openOverlay(name: "policy-help" | "log") {
    window.history.pushState({ ...window.history.state, usbSrPage: activePage(), usbSrOverlay: name }, "");
    if (name === "policy-help") setPolicyHelpOpen(true);
    else setLogOpen(true);
  }

  function closeOverlay(name: "policy-help" | "log", fromHistory = false) {
    if (name === "policy-help") setPolicyHelpOpen(false);
    else setLogOpen(false);
    if (!fromHistory && window.history.state?.usbSrOverlay === name) window.history.back();
  }

  function openConfirm(request: ConfirmRequest) {
    window.history.pushState({ ...window.history.state, usbSrPage: activePage(), usbSrOverlay: "confirm" }, "");
    setConfirmRequest({
      ...request,
      title: tx(request.title),
      message: translateRuntime(language(), request.message),
      confirmLabel: tx(request.confirmLabel)
    });
  }

  function closeConfirm(fromHistory = false) {
    setConfirmRequest(undefined);
    if (!fromHistory && window.history.state?.usbSrOverlay === "confirm") window.history.back();
  }

  function dismissConfirm(fromHistory = false) {
    const cancel = confirmRequest()?.cancel;
    closeConfirm(fromHistory);
    cancel?.();
  }

  function acceptConfirm() {
    const action = confirmRequest()?.action;
    closeConfirm();
    action?.();
  }

  function confirmWebUi(title: string, message: string, confirmLabel = "继续"): Promise<boolean> {
    return new Promise((resolve) => {
      openConfirm({
        title,
        message,
        confirmLabel,
        action: () => resolve(true),
        cancel: () => resolve(false)
      });
    });
  }

  function showToast(message: string, tone?: "success" | "error") {
    const sourceMessage = message;
    message = translateRuntime(language(), message);
    const resolvedTone = tone ?? (
      sourceMessage.toLowerCase().includes("error") || sourceMessage.includes("失败")
        ? "error"
        : sourceMessage.includes("已") || sourceMessage.includes("完成") || sourceMessage.includes("成功")
          ? "success"
          : undefined
    );
    const id = Math.random().toString(36).slice(2);
    setToasts((current) => [...current, { id, message, tone: resolvedTone }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 4200);
  }

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings((current) => ({ ...current, [key]: value }));
  };

  async function refresh(showSuccess = true, manageBusy = true, scope: RefreshScope = "all"): Promise<boolean> {
    if (manageBusy) setBusy(true);
    try {
      const result = await rootExec(`${CONTROLLER} status`);
      if (result.code !== 0) throw new Error(result.stderr || result.stdout || tx("读取状态失败"));
      const parsed = parseStatus(result.stdout);
      setStatus(parsed);
      if (scope === "all" || scope === "policy") setSettings(initialFromStatus(parsed));
      if (scope === "all" || scope === "tools") {
        if (parsed.bluetooth_hal) setBluetoothHal(parsed.bluetooth_hal);
        if (parsed.resampler_preset) setResamplerPreset(parsed.resampler_preset);
        if (parsed.resampler_bypass) setResamplerBypass(parsed.resampler_bypass);
        if (parsed.resampler_cheat) setResamplerCheat(parsed.resampler_cheat === "1");
        if (parsed.resampler_stop_band) setResamplerStopBand(parsed.resampler_stop_band);
        if (parsed.resampler_half_length) setResamplerHalfLength(parsed.resampler_half_length);
        if (parsed.resampler_percent) setResamplerPercent(parsed.resampler_percent);
        if (parsed.usb_period) setUsbPeriod(normalizeUsbPeriod(parsed.usb_period));
        if (parsed.diagnostic) setDiagnostic(parsed.diagnostic);
        if (parsed.diagnostic_all) setDiagnosticAll(parsed.diagnostic_all === "1");
      }
      if (scope === "all" || scope === "tuning") {
        if (parsed.io_scheduler) setIoScheduler(parsed.io_scheduler);
        if (parsed.io_tone) setIoTone(parsed.io_tone);
        if (parsed.wifi_no_restart) setWifiNoRestart(parsed.wifi_no_restart === "1");
        setJitterValues(Object.fromEntries(jitterFeatures.map(([feature]) => [
          feature,
          parsed[`jitter_${feature}`] === "1"
        ])) as JitterValues);
        setJitterDirty([]);
      }
      setAutoReapply(parsed.auto_reapply === "1");
      if (showSuccess) showToast(tx("状态已更新"));
      return true;
    } catch (error) {
      showToast(String(error), "error");
      return false;
    } finally {
      if (manageBusy) setBusy(false);
    }
  }

  async function confirmApplyWithConnectedA2dp(): Promise<boolean> {
    const result = await rootExec(`${CONTROLLER} status`);
    if (result.code !== 0) throw new Error(result.stderr || result.stdout || tx("读取 A2DP 状态失败"));
    const liveStatus = parseStatus(result.stdout);
    setStatus(liveStatus);
    if (liveStatus.bluetooth_a2dp_connected !== "1") return true;
    return confirmWebUi(
      "蓝牙音频正在使用",
      "当前检测到 A2DP 蓝牙音频设备已连接，继续应用可能导致当前蓝牙音频连接失效；若失效，需要先恢复可用策略模板，并断开重新连接蓝牙设备才可以正常恢复蓝牙音频。",
      "继续应用"
    );
  }

  async function handleA2dpFailure(action: string) {
    const openSettings = await confirmWebUi(
      "A2DP 路由异常",
      `${action}已完成，但 A2DP 路由检查失败，蓝牙媒体音频可能已经失效。需要打开蓝牙设置以断开并重新连接设备。`,
      "打开蓝牙设置"
    );
    if (!openSettings) {
      showToast("A2DP 路由检查失败，请手动断开并重新连接蓝牙音频设备", "error");
      return;
    }
    const result = await rootExec("am start -a android.settings.BLUETOOTH_SETTINGS");
    if (result.code !== 0) throw new Error(result.stderr || result.stdout || tx("无法打开蓝牙设置"));
    showToast(tx("已打开蓝牙设置，请断开并重新连接音频设备"));
  }

  async function apply() {
    const current = settings();
    const numericRate = Number(rateValue(current));
    if (!Number.isInteger(numericRate) || numericRate < 44100 || numericRate > 768000) {
      showToast(tx("采样率必须是 44100–768000 Hz 的整数"), "error");
      return;
    }
    setBusy(true);
    try {
      if (isAidl() && !await confirmWebUi(
        "应用 AIDL 音频策略",
        "将把已校验的候选 XML 临时挂载到实际配置来源，并重启 AIDL Audio HAL 与 audioserver。当前音频会短暂中断；若服务或运行时 profile 校验失败，控制器会自动卸载并恢复原始配置。",
        "确认应用"
      )) return;
      if (!await confirmApplyWithConnectedA2dp()) return;
      showToast(tx("正在应用更改，音频将会短暂断开"));
      const command = `${CONTROLLER} apply ${controllerArgs(current)}`;
      const result = await rootExec(command);
      const text = `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
      setLog(text || `exit=${result.code}`);
      if (result.code === 72) {
        await handleA2dpFailure("音频策略");
        await refresh(false, false, "policy");
        return;
      }
      if (result.code !== 0) throw new Error(text || `应用失败，退出码 ${result.code}`);
      const refreshed = await refresh(false, false, "policy");
      if (refreshed) showToast(result.stdout.includes("bluetooth_a2dp_before=1") ? tx("配置已应用，A2DP 路由正常") : tx("配置已应用"));
    } catch (error) {
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  async function previewAidl() {
    const current = settings();
    const numericRate = Number(rateValue(current));
    if (!Number.isInteger(numericRate) || numericRate < 44100 || numericRate > 768000) {
      showToast(tx("采样率必须是 44100–768000 Hz 的整数"), "error");
      return;
    }
    setBusy(true);
    try {
      const command = `${CONTROLLER} aidl-preview ${[
        current.policy,
        String(numericRate),
        current.bitDepth
      ].map(shellQuote).join(" ")}`;
      const result = await rootExec(command);
      const text = `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
      setLog(text || `exit=${result.code}`);
      if (result.code !== 0) throw new Error(text || `AIDL 预览失败，退出码 ${result.code}`);
      openOverlay("log");
      showToast(tx("AIDL 候选配置预览已生成"), "success");
    } catch (error) {
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  function changePolicy(value: string) {
    if (!isAidl()) {
      update("policy", value);
      return;
    }
    const supported = status()[`aidl_supported_rates_${value.replaceAll("-", "_")}`]
      ?.split(",")
      .filter(Boolean) ?? [];
    setSettings((current) => {
      const currentRate = rateValue(current);
      const nextRate = supported.length && !supported.includes(currentRate) ? supported[0] : currentRate;
      const known = rateOptions.some(([rate]) => rate === nextRate);
      return {
        ...current,
        policy: value,
        rate: known ? nextRate : "custom",
        customRate: nextRate
      };
    });
  }

  async function reset() {
    setBusy(true);
    showToast(tx("正在重置…"));
    try {
      const result = await rootExec(`${CONTROLLER} reset`);
      const text = `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
      setLog(text || `exit=${result.code}`);
      if (result.code === 72) {
        await handleA2dpFailure("重置");
        await refresh(false, false, "policy");
        return;
      }
      if (result.code !== 0) throw new Error(text || `重置失败，退出码 ${result.code}`);
      const refreshed = await refresh(false, false, "policy");
      if (refreshed) showToast(tx("已重置"));
    } catch (error) {
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  async function runExtra(args: string[], _pending: string, success: string) {
    const action: ToolAction = args[0] === "resampler" && args[1] === "reset"
      ? "resampler-reset"
      : args[0] === "usb-period" && args[1] === "reset"
        ? "usb-period-reset"
        : args[0] === "jitter" && args[1] === "disable" && args[2] === "all"
          ? "jitter-reset"
          : args[0] as ToolAction;
    setToolAction(action);
    setBusy(true);
    try {
      if (["bluetooth-hal", "resampler", "usb-period"].includes(args[0])) {
        showToast(tx("正在应用更改，音频将会短暂断开"));
      }
      const command = `${CONTROLLER} extra ${args.map(shellQuote).join(" ")}`;
      const result = await rootExec(command);
      const text = `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
      setLog(text || `exit=${result.code}`);
      if (result.code === 72) {
        await handleA2dpFailure("设置");
        return;
      }
      if (result.code !== 0) throw new Error(text || `执行失败，退出码 ${result.code}`);
      // Refresh the affected controls before announcing success. This keeps the
      // completion toast and the busy-state release on the same status check.
      const refreshed = await refresh(false, false, args[0] === "jitter" ? "tuning" : "tools");
      if (refreshed) showToast(tx(success));
    } catch (error) {
      showToast(String(error), "error");
    } finally {
      setBusy(false);
      setToolAction(undefined);
    }
  }

  function normalizeUsbPeriod(value = usbPeriod()): string {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) return "2250";
    return String(Math.min(50000, Math.max(125, Math.round(numeric / 125) * 125)));
  }

  function stepUsbPeriod(direction: -1 | 1) {
    setUsbPeriod(normalizeUsbPeriod(String(Number(normalizeUsbPeriod()) + direction * 125)));
  }

  function keepRangeThumbOnly(event: PointerEvent) {
    const input = event.currentTarget as HTMLInputElement;
    delete input.dataset.blockedClick;
    delete input.dataset.blockedValue;
    const rect = input.getBoundingClientRect();
    const min = Number(input.min);
    const max = Number(input.max);
    const value = Number(input.value);
    if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min || !Number.isFinite(value)) return;
    const inset = Math.min(12, rect.height / 2);
    const fraction = Math.max(0, Math.min(1, (value - min) / (max - min)));
    const thumbCenter = rect.left + inset + fraction * Math.max(0, rect.width - inset * 2);
    if (Math.abs(event.clientX - thumbCenter) > Math.max(14, inset * 1.6)) {
      input.dataset.blockedClick = "true";
      input.dataset.blockedValue = input.value;
      event.preventDefault();
      event.stopPropagation();
    }
  }

  function handleRangeInput(event: InputEvent & { currentTarget: HTMLInputElement }) {
    const range = event.currentTarget;
    if (range.dataset.blockedClick === "true") {
      range.value = range.dataset.blockedValue ?? range.value;
      return;
    }
    setUsbPeriod(range.value);
  }

  function releaseRangePointer(event: Event) {
    const range = event.currentTarget as HTMLInputElement;
    if (range.dataset.blockedClick !== "true") return;
    range.value = range.dataset.blockedValue ?? range.value;
    window.setTimeout(() => {
      delete range.dataset.blockedClick;
      delete range.dataset.blockedValue;
    }, 0);
  }

  function cancelBlockedRangeClick(event: MouseEvent) {
    const range = event.currentTarget as HTMLInputElement;
    if (range.dataset.blockedClick !== "true") return;
    event.preventDefault();
    event.stopPropagation();
    delete range.dataset.blockedClick;
    delete range.dataset.blockedValue;
  }

  function applyUsbPeriod() {
    const normalized = normalizeUsbPeriod();
    if (normalized !== usbPeriod()) setUsbPeriod(normalized);
    void runExtra(["usb-period", normalized], "正在应用 USB period…", "USB period 已应用");
  }

  function normalizeResamplerValue(value: string, min: number, max: number, step: number, fallback: string): string {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) return fallback;
    return String(Math.min(max, Math.max(min, min + Math.round((numeric - min) / step) * step)));
  }

  function normalizeResamplerStopBand(value = resamplerStopBand()): string {
    return normalizeResamplerValue(value, 20, 242, 1, "179");
  }

  function normalizeResamplerHalfLength(value = resamplerHalfLength()): string {
    return normalizeResamplerValue(value, 8, 640, 8, "408");
  }

  function normalizeResamplerPercent(value = resamplerPercent(), cheat = resamplerCheat()): string {
    return normalizeResamplerValue(value, 1, cheat ? 200 : 100, 1, "99");
  }

  function changeRate(value: string) {
    update("rate", value);
    if (value !== "custom") return;
    window.setTimeout(() => openConfirm({
      title: "自定义采样率提示",
      message: "自定义采样率可能不受当前 DAC、USB Audio HAL 或音频策略支持。应用后如出现无声、失真或播放失败，请改用设备明确支持的采样率。",
      confirmLabel: "知道了",
      action: () => undefined,
      showCancel: false
    }), 220);
  }

  function applyLanguage() {
    const nextLanguage = pendingLanguage();
    setLanguage(nextLanguage);
    window.localStorage.setItem("usbSrLanguage", nextLanguage);
  }

  async function applyAutoReapply(value: boolean) {
    setBusy(true);
    try {
      const result = await rootExec(`${CONTROLLER} settings auto-reapply ${value ? "enable" : "disable"}`);
      if (result.code !== 0) throw new Error(result.stderr || result.stdout || tx("实验选项保存失败"));
      setAutoReapply(value);
      showToast(tx(value ? "启动自动应用已启用" : "启动自动应用已停用"));
    } catch (error) {
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  async function changeAutoReapply(value: boolean) {
    if (value && !await confirmWebUi(
      "启用启动自动应用",
      "系统启动完成后将重新应用已保存的策略、工具和调优设置，其中可能包含允许 SELinux Permissive、停用温控等高风险选项。请确认已了解风险。",
      "确认启用"
    )) return;
    await applyAutoReapply(value);
  }

  async function runDiagnostic() {
    setBusy(true);
    setDiagnosticOutput(tx("正在读取设备诊断信息…"));
    try {
      const args = ["diagnose", diagnostic(), ...(diagnosticAll() ? ["all"] : [])];
      const result = await rootExec(`${CONTROLLER} extra ${args.map(shellQuote).join(" ")}`);
      const text = `${result.stdout}${result.stderr ? `\n[stderr]\n${result.stderr}` : ""}`.trim();
      setDiagnosticOutput(text || `exit=${result.code}`);
      if (result.code !== 0) throw new Error(`诊断失败，退出码 ${result.code}`);
    } catch (error) {
      setDiagnosticOutput((current) => current === tx("正在读取设备诊断信息…") ? translateRuntime(language(), String(error)) : current);
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  function updateJitter(feature: JitterFeature, value: boolean) {
    setJitterValues((current) => ({ ...current, [feature]: value }));
    setJitterDirty((current) => current.includes(feature) ? current : [...current, feature]);
  }

  async function applyJitter() {
    const dirty = jitterDirty();
    if (!dirty.length) {
      showToast(tx("没有待应用的 jitter reducer 修改"));
      return;
    }
    if ((dirty.includes("selinux") && jitterValues().selinux) || (dirty.includes("thermal") && jitterValues().thermal)) {
      const riskActions = [
        dirty.includes("selinux") && jitterValues().selinux ? tx("允许 SELinux Permissive") : "",
        dirty.includes("thermal") && jitterValues().thermal ? tx("停用系统温控") : ""
      ].filter(Boolean).join(language() === "en" ? ", and " : "，并");
      if (!await confirmWebUi(
        "确认高风险操作",
        language() === "en"
          ? `This will ${riskActions}. It may reduce system security and stability and could cause overheating or damage. Confirm that you understand the risk.`
          : `即将${riskActions}。这可能降低系统安全性和稳定性，并可能导致设备过热或损坏。请确认已了解风险。`,
        "接受风险并应用"
      )) return;
    }
    setBusy(true);
    const outputs: string[] = [];
    try {
      if (dirty.includes("effect")) {
        showToast(tx("正在应用更改，音频将会短暂断开"));
      }
      for (const feature of dirty) {
        const args = ["jitter", jitterValues()[feature] ? "enable" : "disable", feature];
        if (feature === "io" && jitterValues().io) args.push(ioScheduler(), ioTone());
        if (feature === "wifi" && jitterValues().wifi && wifiNoRestart()) args.push("no-restart");
        const result = await rootExec(`${CONTROLLER} extra ${args.map(shellQuote).join(" ")}`);
        outputs.push(`[${feature}]\n${result.stdout}${result.stderr}`.trim());
        if (result.code !== 0) throw new Error(`${feature} 执行失败，退出码 ${result.code}`);
        setJitterDirty((current) => current.filter((item) => item !== feature));
      }
      setLog(outputs.join("\n\n"));
      showToast(tx("Jitter reducer 设置已应用"));
      await refresh(false, false, "tuning");
    } catch (error) {
      setLog(outputs.join("\n\n"));
      showToast(String(error), "error");
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    window.history.replaceState({ ...window.history.state, usbSrPage: activePage() }, "");
    const handlePopState = () => {
      if (confirmRequest()) {
        dismissConfirm(true);
        return;
      }
      if (policyHelpOpen()) {
        closeOverlay("policy-help", true);
        return;
      }
      if (logOpen()) {
        closeOverlay("log", true);
        return;
      }
    };
    window.addEventListener("popstate", handlePopState);
    onCleanup(() => window.removeEventListener("popstate", handlePopState));

    if (pageViewport) {
      carousel = EmblaCarousel(pageViewport, {
        align: "start",
        containScroll: "trimSnaps",
        duration: 18,
        loop: false,
        skipSnaps: false,
        watchDrag: (_api, event) => !(event.target instanceof Element && (
          event.target.closest("[data-no-page-drag]") || event.target.closest(".page-swipe-input")
        ))
      });
      carousel.on("scroll", syncCarousel);
      carousel.on("select", syncCarousel);
      carousel.on("pointerDown", () => setPageDragging(true));
      carousel.on("pointerUp", () => setPageDragging(false));
      carousel.on("settle", () => {
        setPageDragging(false);
        syncCarousel();
      });
      syncCarousel();
    }
    void refresh(false);
  });

  onCleanup(() => carousel?.destroy());

  return (
    <>
      <div class="app-shell">
        <header class="app-header">
          <div class="brand">
            <span class="brand-mark"><BrandIcon /></span>
            <span class="brand-copy"><strong>SampleRate Changer</strong><small>{tx("Root audio policy")}</small></span>
          </div>
          <button class="icon-button" aria-label={tx("刷新状态")} title={tx("刷新状态")} onClick={() => void refresh()} disabled={busy()}><RefreshIcon /></button>
          <button class="icon-button" aria-label={tx("打开执行日志")} title={tx("执行日志")} onClick={() => openOverlay("log")}><LogIcon /></button>
        </header>

        <div class="page-viewport" ref={pageViewport} onTouchStart={handleInputTouchStart} onTouchEnd={handleInputTouchEnd} onTouchCancel={() => { inputSwipeStart = undefined; }}>
          <div class="page-track">
            <section class="page-panel" data-page="policy" aria-label={tx("音频策略页面")}>
              <main class="page-content">
                <section class="card preview-card">
                  <div class="section-heading"><h2>{tx("本次执行")}</h2></div>
                  <div class="summary-line"><span class="summary-key">{tx("策略")}</span><strong>{tx(policyLabel(settings().policy, policyOptions.find(([value]) => value === settings().policy)?.[1] ?? ""))}</strong><span class="summary-key">{tx("格式")}</span><strong>{tx(displayRate(rateValue(settings())))} · {tx(bitOptions.find(([value]) => value === settings().bitDepth)?.[1] ?? "")}</strong></div>
                  <div class="tag-row"><Show when={settings().drc}><span class="tag">DRC</span></Show><Show when={settings().forceUsbv2}><span class="tag">USBv2</span></Show><Show when={settings().forceBluetoothQti}><span class="tag">Bluetooth QTI</span></Show></div>
                  <Show when={isAidl()}><div class="notice"><WarningIcon /><span>{tx("当前为 AIDL Audio HAL。你可以先生成只读预览；点击应用后会挂载已校验的候选配置并重启音频服务，失败时自动恢复原始配置。")}</span></div></Show>
                  <div class={`preview-actions ${isAidl() ? "aidl" : ""}`}><button class="secondary-button" onClick={() => openConfirm({ title: "重置音频策略", message: isAidl() ? "将卸载 AIDL module XML，并重启 Audio HAL 与 audioserver 恢复原始配置。当前音频会短暂中断。" : "将卸载生成的 audio policy bind mount，并重启音频服务。当前音频连接可能会短暂中断。", confirmLabel: "确认重置", action: () => void reset() })} disabled={busy()}>{tx("重置")}</button><Show when={isAidl()}><button class="secondary-button" onClick={() => void previewAidl()} disabled={busy() || status().aidl_preview_supported !== "1"}>{tx("生成只读预览")}</button></Show><button class="primary-button" onClick={() => void apply()} disabled={busy() || (isAidl() && status().aidl_apply_supported !== "1")}>{busy() ? tx("处理中…") : tx("应用")}</button></div>
                </section>

                <section class="grid two-col">
                  <article class="card section-card">
                    <SectionHeading title={tx("音频策略")} />
                    <div class="field-label-row"><label class="field-label" for="policy">{tx("策略模板")}</label><button type="button" class="inline-link" onClick={() => openOverlay("policy-help")}>ⓘ {tx("说明")}</button></div>
                    <SelectField id="policy" title={tx("选择策略模板")} value={settings().policy} options={localizePolicyOptions()} language={language()} onChange={changePolicy} />
                  </article>

                  <article class="card section-card">
                    <SectionHeading title={tx("采样格式")} />
                    <label class="field-label" for="rate">{tx("采样率")}</label>
                    <SelectField id="rate" title={tx("选择采样率")} value={settings().rate} options={localizeOptions(availableRateOptions())} language={language()} onChange={changeRate} />
                    <Show when={settings().rate === "custom"}><input class="number-input spaced-input page-swipe-input" inputmode="numeric" type="number" min="44100" max="768000" step="1" value={settings().customRate} onInput={(event) => update("customRate", event.currentTarget.value)} placeholder={tx("例如 123456")} /></Show>
                    <label class="field-label" for="bits">{tx("位深 / 格式")}</label>
                    <SelectField id="bits" title={tx("选择位深与格式")} value={settings().bitDepth} options={localizeOptions(bitOptions)} language={language()} onChange={(value) => update("bitDepth", value)} />
                  </article>
                </section>

                <section class="card section-card">
                  <SectionHeading title={tx("功能开关")} />
                  <div class="switch-grid">
                    <ToggleRow label={tx("DRC 动态范围控制")} description={tx("压缩过大的音量动态；USB Only 策略下不会生效。")} checked={settings().drc} onChange={(value) => update("drc", value)} />
                    <ToggleRow label={tx("强制 USBv2 HAL")} description={tx("优先使用 USB Audio HAL v2，仅建议用于兼容性排查。")} checked={settings().forceUsbv2} onChange={(value) => update("forceUsbv2", value)} />
                    <ToggleRow label={tx("强制 Bluetooth QTI")} description={tx("强制使用 Qualcomm bluetooth_qti 路径，非高通设备请勿启用。")} checked={settings().forceBluetoothQti} onChange={(value) => update("forceBluetoothQti", value)} />
                  </div>
                </section>

                <Show when={isAidl()}><section class="card section-card aidl-card">
                  <SectionHeading title={tx("AIDL Audio HAL 适配状态")} />
                  <div class="about-details">
                    <div><span>{tx("接口版本")}</span><strong>{status().aidl_core_version ? `AIDL v${status().aidl_core_version}` : status().aidl_core_versions || "AIDL"}</strong></div>
                    <div><span>{tx("配置适配器")}</span><strong>{status().aidl_config_adapter || "read-only"}</strong></div>
                    <div><span>{tx("配置来源")}</span><strong>{status().aidl_config_path || status().audio_config_source || "—"}</strong></div>
                    <div><span>{tx("拓扑匹配")}</span><strong>{status().aidl_config_topology || "—"}</strong></div>
                    <div><span>{tx("USB 可达端口")}</span><strong>{status().aidl_usb_reachable_mix_port_count || "0"}</strong></div>
                  </div>
                  <p class="field-help">{tx("端口按 flags、动态 profile 和 USB 路由可达性分类，不依赖 hifi_playback 等厂商名字。")}</p>
                  <p class="aidl-behaviors">{status().aidl_usb_port_behaviors || "—"}</p>
                  <Show when={status().aidl_config_error}><p class="field-help">{status().aidl_config_error}</p></Show>
                </section></Show>

                <section class="status-strip card">
                  <div><span class="label">audioserver</span><strong>{status().audioserver_pid || tx("未检测到")}</strong></div>
                  <div><span class="label">Audio HAL</span><strong>{isAidl() ? (status().aidl_core_version ? `AIDL v${status().aidl_core_version}` : "AIDL") : tx("传统 / HIDL")}</strong></div>
                  <div><span class="label">{tx("当前采样率")}</span><strong>{status().sample_rate ? tx(displayRate(status().sample_rate ?? "")) : "—"}</strong></div>
                  <div><span class="label">Bluetooth A2DP</span><strong>{status().bluetooth_a2dp_connected === "1" ? tx("已连接") : tx("未连接")}</strong></div>
                </section>
              </main>
            </section>

            <section class="page-panel" aria-label={tx("扩展工具页面")}>
              <main class="page-content">
                <div class="page-intro"><div class="intro-icon"><ToolsIcon /></div><div><h1>{tx("音频扩展工具")}</h1></div></div>
                <section class="grid two-col extra-grid">
                  <article class="card tool-panel">
                    <SectionHeading title={tx("蓝牙音频 HAL")} />
                    <p class="field-help">{tx("选择系统使用的蓝牙音频实现。应用时会修改持久属性并重启音频 HAL。")}</p>
                    <SelectField title={tx("选择 Bluetooth HAL")} value={bluetoothHal()} options={localizeOptions(bluetoothHalOptions)} language={language()} onChange={(value) => setBluetoothHal(value)} />
                    <div class="inline-actions"><button class="primary-button" disabled={busy()} onClick={() => runExtra(["bluetooth-hal", bluetoothHal()], "正在切换 Bluetooth HAL…", "Bluetooth HAL 已切换")}>{toolAction() === "bluetooth-hal" ? tx("处理中…") : tx("应用")}</button></div>
                  </article>

                  <article class="card tool-panel">
                    <SectionHeading title={tx("AudioFlinger 重采样器")} />
                    <p class="field-help">{tx("选择重采样质量预设，或自定义阻带衰减、滤波器长度与截止比例。")}</p>
                    <SelectField title={tx("选择重采样预设")} value={resamplerPreset()} options={[]} groups={resamplerPresetGroups.map(({ label, options }) => ({ label: tx(label), options: localizeOptions(options) }))} language={language()} onChange={(value) => setResamplerPreset(value)} />
                    <Show when={resamplerPreset() === "custom"}><div class="custom-resampler">
                      <label class="field-label">{tx("生效起始采样率")}</label><SelectField title={tx("选择生效起始采样率")} value={resamplerBypass()} options={localizeOptions(resamplerBypassOptions)} language={language()} onChange={(value) => setResamplerBypass(value)} />
                      <ToggleRow label={tx("补偿模式（Cheat）")} description={tx("关闭时使用标准截止百分比。")} checked={resamplerCheat()} onChange={(value) => { setResamplerCheat(value); setResamplerPercent(normalizeResamplerPercent(resamplerPercent(), value)); }} />
                      <label class="field-label">{tx("阻带衰减")} (dB)<input class="number-input page-swipe-input" aria-label={`${tx("阻带衰减")} (dB)`} inputmode="numeric" type="number" min="20" max="242" step="1" value={resamplerStopBand()} onInput={(event) => setResamplerStopBand(event.currentTarget.value)} onBlur={() => setResamplerStopBand(normalizeResamplerStopBand())} disabled={busy()} /></label>
                      <label class="field-label">{tx("半滤波器长度")}<input class="number-input page-swipe-input" aria-label={tx("半滤波器长度")} inputmode="numeric" type="number" min="8" max="640" step="8" value={resamplerHalfLength()} onInput={(event) => setResamplerHalfLength(event.currentTarget.value)} onBlur={() => setResamplerHalfLength(normalizeResamplerHalfLength())} disabled={busy()} /></label>
                      <label class="field-label">{resamplerCheat() ? tx("补偿百分比（Cheat）") : tx("截止百分比")} (%)<input class="number-input page-swipe-input" aria-label={`${resamplerCheat() ? tx("补偿百分比（Cheat）") : tx("截止百分比")} (%)`} inputmode="numeric" type="number" min="1" max={resamplerCheat() ? "200" : "100"} step="1" value={resamplerPercent()} onInput={(event) => setResamplerPercent(event.currentTarget.value)} onBlur={() => setResamplerPercent(normalizeResamplerPercent())} disabled={busy()} /></label>
                    </div></Show>
                    <div class="inline-actions"><button class="secondary-button" disabled={busy()} onClick={() => openConfirm({ title: "重置重采样设置", message: "将清除 AudioFlinger 重采样属性并恢复系统默认行为。", confirmLabel: "确认重置", action: () => void runExtra(["resampler", "reset"], "正在重置重采样…", "重采样已恢复系统默认") })}>{toolAction() === "resampler-reset" ? tx("处理中…") : tx("重置")}</button><button class="primary-button" disabled={busy()} onClick={() => runExtra(resamplerPreset() === "custom" ? ["resampler", "custom", resamplerBypass(), resamplerCheat() ? "cheat" : "cutoff", normalizeResamplerStopBand(), normalizeResamplerHalfLength(), normalizeResamplerPercent()] : ["resampler", resamplerPreset()], "正在应用重采样配置…", "重采样配置已应用")}>{toolAction() === "resampler" ? tx("处理中…") : tx("应用")}</button></div>
                  </article>

                  <article class="card tool-panel">
                    <SectionHeading title={tx("USB 传输周期")} />
                    <p class="field-help">{tx("设置 USB 音频数据传输间隔。数值越低延迟越小，但可能降低播放稳定性。")}</p>
                    <div class="period-control">
                      <div class="period-stepper">
                        <button type="button" data-no-page-drag aria-label={tx("减少 125 微秒")} onClick={() => stepUsbPeriod(-1)} disabled={busy() || Number(normalizeUsbPeriod()) <= 125}>−</button>
                        <label><input class="page-swipe-input" aria-label={tx("USB Transfer Period")} inputmode="numeric" type="number" min="125" max="50000" step="125" value={usbPeriod()} onInput={(event) => setUsbPeriod(event.currentTarget.value)} onBlur={() => setUsbPeriod(normalizeUsbPeriod())} /><span>μs</span></label>
                        <button type="button" data-no-page-drag aria-label={tx("增加 125 微秒")} onClick={() => stepUsbPeriod(1)} disabled={busy() || Number(normalizeUsbPeriod()) >= 50000}>+</button>
                      </div>
                      <input class="period-range" data-no-page-drag aria-label={tx("快速调整 USB Transfer Period")} type="range" min="125" max="50000" step="125" value={normalizeUsbPeriod()} onPointerDown={keepRangeThumbOnly} onInput={handleRangeInput} onPointerUp={releaseRangePointer} onPointerCancel={releaseRangePointer} onClick={cancelBlockedRangeClick} />
                      <div class="period-scale"><span>125 μs</span><strong>{normalizeUsbPeriod()} μs</strong><span>50000 μs</span></div>
                    </div>
                    <div class="inline-actions"><button class="secondary-button" disabled={busy()} onClick={() => openConfirm({ title: "重置 USB Transfer Period", message: "将清除当前 USB 传输周期设置并恢复系统默认值。", confirmLabel: "确认重置", action: () => void runExtra(["usb-period", "reset"], "正在重置 USB period…", "USB period 已恢复系统默认") })}>{toolAction() === "usb-period-reset" ? tx("处理中…") : tx("重置")}</button><button class="primary-button" disabled={busy()} onClick={applyUsbPeriod}>{toolAction() === "usb-period" ? tx("处理中…") : tx("应用")}</button></div>
                  </article>

                  <article class="card tool-panel">
                    <SectionHeading title={tx("音频诊断")} />
                    <p class="field-help">{tx("读取 AudioFlinger、蓝牙编解码、音频配置文件或 ALSA 设备状态。")}</p>
                    <SelectField title={tx("选择诊断类型")} value={diagnostic()} options={localizeOptions(diagnosticOptions)} language={language()} onChange={(value) => setDiagnostic(value)} />
                    <ToggleRow label={tx("完整输出")} description={tx("关闭时只保留关键字段。")} checked={diagnosticAll()} onChange={setDiagnosticAll} />
                    <div class="inline-actions"><button class="primary-button" disabled={busy()} onClick={runDiagnostic}>{tx("运行诊断")}</button></div>
                    <pre class="diagnostic-output" aria-live="polite">{diagnosticOutput() === "尚未运行诊断。" ? tx(diagnosticOutput()) : diagnosticOutput()}</pre>
                  </article>
                </section>
              </main>
            </section>

            <section class="page-panel" aria-label={tx("系统调优页面")}>
              <main class="page-content">
                <div class="page-intro danger-intro"><div class="intro-icon"><TuneIcon /></div><div><h1>{tx("系统 Jitter 优化")}</h1></div></div>
                <section class="card section-card danger-card">
                  <div class="notice danger-notice"><WarningIcon /><span><strong>{tx("风险提示：")}</strong>{tx("这些选项可能修改 SELinux、温控、系统服务、调度器和内核参数，可能降低系统安全性、稳定性或导致设备过热。请仅启用已了解影响的项目；SELinux 与温控选项应用时会再次确认。")}</span></div>
                  <div class="switch-grid jitter-grid"><For each={jitterFeatures}>{(feature) => <ToggleRow label={`${tx(feature[1])}${jitterDirty().includes(feature[0]) ? tx(" · 待应用") : ""}`} description={tx(feature[2])} checked={jitterValues()[feature[0]]} onChange={(value) => updateJitter(feature[0], value)} />}</For></div>
                  <Show when={jitterValues().io}><div class="grid two-col io-options"><div><label class="field-label">I/O scheduler</label><SelectField title={tx("选择 I/O scheduler")} value={ioScheduler()} options={localizeOptions(ioSchedulerOptions)} language={language()} onChange={(value) => { setIoScheduler(value); setJitterDirty((current) => current.includes("io") ? current : [...current, "io"]); }} /></div><div><label class="field-label">{tx("声音倾向")}</label><SelectField title={tx("选择声音倾向")} value={ioTone()} options={ioToneOptions} language={language()} onChange={(value) => { setIoTone(value); setJitterDirty((current) => current.includes("io") ? current : [...current, "io"]); }} /></div></div></Show>
                  <Show when={jitterValues().wifi}><div class="wifi-option"><ToggleRow label={tx("切换时不重启 Wi-Fi")} description={tx("使用 upstream 的 no-restart 模式。")} checked={wifiNoRestart()} onChange={(value) => { setWifiNoRestart(value); setJitterDirty((current) => current.includes("wifi") ? current : [...current, "wifi"]); }} /></div></Show>
                  <div class="inline-actions right tuning-actions"><button class="secondary-button" disabled={busy()} onClick={() => openConfirm({ title: "重置 Jitter 基础项", message: "将恢复 SELinux、温控、Doze、调频、相机、日志、I/O、虚拟内存和 Wi-Fi 的基础设置。", confirmLabel: "确认重置", action: () => void runExtra(["jitter", "disable", "all"], "正在重置全部基础 jitter 项…", "Jitter 基础项已重置") })}>{toolAction() === "jitter-reset" ? tx("处理中…") : tx("重置")}</button><button class="primary-button" disabled={busy() || !jitterDirty().length} onClick={applyJitter}>{toolAction() === "jitter" ? tx("处理中…") : tx("应用")}</button></div>
                </section>

              </main>
            </section>

            <section class="page-panel" aria-label={tx("设置页面")}>
              <main class="page-content">
                <div class="page-intro"><div class="intro-icon"><SettingsIcon /></div><div><h1>{tx("设置")}</h1></div></div>
                <section class="card section-card">
                  <SectionHeading title={tx("语言")} />
                  <p class="field-help">{tx("选择 WebUI 显示语言。")}</p>
                  <div class="language-setting"><SelectField title={tx("选择界面语言")} value={pendingLanguage()} options={localizeOptions(languageOptions)} language={language()} onChange={(value) => setPendingLanguage(value as Language)} /><button type="button" class="primary-button" onClick={applyLanguage}>{tx("应用")}</button></div>
                </section>

                <section class="card section-card">
                  <SectionHeading title={tx("实验选项")} />
                  <ToggleRow
                    label={tx("启动时自动重新应用")}
                    description={tx("系统启动完成后重新应用已保存的策略、工具和调优设置。此功能仍处于实验阶段。")}
                    checked={autoReapply()}
                    onChange={(value) => void changeAutoReapply(value)}
                  />
                </section>

                <section class="card section-card about-card">
                  <SectionHeading title={tx("关于")} />
                  <div class="about-brand"><span class="brand-mark"><BrandIcon /></span><div><h3>USB SampleRate Changer WebUI</h3><p>{tx("Root audio policy 工具界面")}</p></div></div>
                  <div class="about-details"><div><span>{tx("模块版本")}</span><strong>{WEBUI_VERSION}</strong></div></div>
                  <p class="about-license">{tx("本项目基于 USB_SampleRate_Changer，遵循项目所附许可证。")}</p>
                </section>
              </main>
            </section>
          </div>
        </div>

        <nav
          class="page-navigation"
          classList={{ dragging: pageDragging() }}
          style={{ "--page-progress": pageProgress() }}
          aria-label={tx("主要页面")}
        >
          <span class="page-nav-indicator" aria-hidden="true" />
          <For each={pageItems}>{(item, index) => {
            const PageIcon = item.icon;
            return <button class="page-nav-item" classList={{ active: activePage() === item.id }} aria-current={activeIndex() === index() ? "page" : undefined} onClick={() => activatePage(item.id)}><PageIcon /><span>{tx(item.label)}</span></button>;
          }}</For>
        </nav>
      </div>

      <ToastHost toasts={toasts()} />

      <Show when={confirmRequest()} keyed>{(request) =>
        <div class="dialog-layer confirm-layer" data-no-page-drag role="presentation">
          <button class="dialog-backdrop" aria-label={tx("取消确认")} onClick={() => dismissConfirm()} />
          <section class="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-message">
            <div class="confirm-mark" aria-hidden="true"><WarningIcon /></div>
            <div><h2 id="confirm-title">{request.title}</h2><p id="confirm-message">{request.message}</p></div>
            <div class="confirm-actions" classList={{ single: request.showCancel === false }}><Show when={request.showCancel !== false}><button type="button" class="secondary-button" onClick={() => dismissConfirm()}>{tx("取消")}</button></Show><button type="button" class="danger-button" onClick={acceptConfirm}>{request.confirmLabel}</button></div>
          </section>
        </div>
      }</Show>

      <Show when={policyHelpOpen()}>
        <div class="dialog-layer" data-no-page-drag role="presentation">
          <button class="dialog-backdrop" aria-label={tx("关闭模板说明")} onClick={() => closeOverlay("policy-help")} />
          <section class="info-dialog" role="dialog" aria-modal="true" aria-labelledby="policy-help-title">
            <header><div><h2 id="policy-help-title">{tx("策略模板说明")}</h2><p>{tx("模板决定生成 audio policy XML 时保留或绕过哪些输出路径；最终结果仍受 ROM、HAL 和设备能力影响。")}</p></div><button class="dialog-close" aria-label={tx("关闭模板说明")} onClick={() => closeOverlay("policy-help")}>×</button></header>
            <div class="policy-guide-list">
              <For each={visiblePolicyOptions()}>{([value, label, summary]) => <article classList={{ current: value === settings().policy }}><div><h3>{tx(policyLabel(value, label))}</h3><Show when={value === settings().policy}><span>{tx("当前选择")}</span></Show></div><p class="policy-summary">{tx(policySummary(value, summary))}</p><p>{tx(policyDescription(value))}</p></article>}</For>
            </div>
          </section>
        </div>
      </Show>

      <Show when={logOpen()}>
        <div class="dialog-layer" role="presentation">
          <button class="dialog-backdrop" aria-label={tx("关闭执行日志")} onClick={() => closeOverlay("log")} />
          <section class="log-dialog" role="dialog" aria-modal="true" aria-labelledby="log-title">
            <header><div><h2 id="log-title">{tx("执行日志")}</h2></div><button class="icon-button" aria-label={tx("关闭执行日志")} onClick={() => closeOverlay("log")}>×</button></header>
            <pre>{log() || tx("暂无执行输出。运行状态查询、诊断或应用配置后会显示在这里。")}</pre>
          </section>
        </div>
      </Show>
    </>
  );
}

function ToastHost(props: { toasts: ToastItem[] }) {
  return <div id="toast-container" aria-live="polite"><For each={props.toasts}>{(toast) => <div class={`toast${toast.tone ? ` ${toast.tone}` : ""}`}>{toast.message}</div>}</For></div>;
}

function SectionHeading(props: { title: string }) {
  return <div class="section-heading"><h2>{props.title}</h2></div>;
}

function ToggleRow(props: { label: string; description: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <label class={`switch-row ${props.disabled ? "disabled" : ""}`}><span class="switch-copy"><strong>{props.label}</strong><small>{props.description}</small></span><input type="checkbox" checked={props.checked} disabled={props.disabled} onChange={(event) => { const input = event.currentTarget; props.onChange(input.checked); input.checked = props.checked; }} /><span class="switch-track"><span /></span></label>;
}

render(() => <App />, document.getElementById("app")!);
