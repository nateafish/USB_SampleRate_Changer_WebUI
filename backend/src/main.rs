use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod aidl;

const CONTROLLER_VERSION: &str = env!("CARGO_PKG_VERSION");
const STATE_ROOT: &str = "/data/adb/usb_samplerate_changer_webui";
const SETTINGS_VERSION: &str = "3";
const CORE_DIR: &str = "core";

const JITTER_FEATURES: &[&str] = &[
    "selinux", "thermal", "doze", "governor", "camera", "logd", "io", "vm", "wifi", "battery",
    "effect",
];
const JITTER_BASE_FEATURES: &[&str] = &[
    "selinux", "thermal", "doze", "governor", "camera", "logd", "io", "vm", "wifi",
];
const IO_SCHEDULERS: &[&str] = &[
    "*",
    "none",
    "noop",
    "deadline",
    "mq-deadline",
    "cfq",
    "bfq",
    "kyber",
];
const IO_TONES: &[&str] = &["light", "m-light", "medium", "boost", "exp"];

const POLICIES: &[(&str, &str, &str)] = &[
    ("auto", "--auto", "自动检测"),
    ("offload", "--offload", "USB 与蓝牙硬件卸载"),
    (
        "offload-hifi-playback",
        "--offload-hifi-playback",
        "USB hifi_playback 硬件卸载",
    ),
    (
        "offload-direct",
        "--offload-direct",
        "Direct PCM / 压缩卸载",
    ),
    ("offload-safer", "--offload-safer", "较安全的 USB 硬件卸载"),
    ("bypass", "--bypass-offload", "绕过 USB 与蓝牙硬件卸载"),
    (
        "bypass-safer",
        "--bypass-offload-safer",
        "较安全地绕过 USB 与蓝牙硬件卸载",
    ),
    ("legacy", "--legacy", "旧版 A2DP HAL"),
    ("safe", "--safe", "安全兼容模式"),
    ("safest", "--safest", "最安全兼容模式"),
    ("safest-auto", "--safest-auto", "最安全并自动检测 USB 上限"),
    ("usb", "--usb-only", "仅修改 USB 音频策略"),
];

const DOCUMENTED_RATES: &[(u32, &str)] = &[
    (44_100, "44.1 kHz"),
    (48_000, "48 kHz"),
    (88_200, "88.2 kHz"),
    (96_000, "96 kHz"),
    (176_400, "176.4 kHz"),
    (192_000, "192 kHz"),
    (352_800, "352.8 kHz"),
    (384_000, "384 kHz"),
    (705_600, "705.6 kHz"),
    (768_000, "768 kHz"),
];

const BIT_DEPTHS: &[(&str, &str)] = &[
    ("16", "16-bit PCM"),
    ("24", "24-bit packed PCM"),
    ("32", "32-bit PCM"),
    ("float", "32-bit float PCM"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Settings {
    policy: String,
    sample_rate: u32,
    bit_depth: String,
    drc: bool,
    force_usbv2: bool,
    force_bluetooth_qti: bool,
    amzm: bool,
    test: bool,
    test_template: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            policy: "auto".to_string(),
            sample_rate: 44_100,
            bit_depth: "32".to_string(),
            drc: false,
            force_usbv2: false,
            force_bluetooth_qti: false,
            amzm: false,
            test: false,
            test_template: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredSettings {
    policy: Settings,
    policy_configured: bool,
    bluetooth_hal: String,
    bluetooth_hal_configured: bool,
    resampler_preset: String,
    resampler_configured: bool,
    resampler_bypass: String,
    resampler_cheat: bool,
    resampler_stop_band: u32,
    resampler_half_length: u32,
    resampler_percent: u32,
    usb_period: u32,
    usb_period_configured: bool,
    diagnostic: String,
    diagnostic_all: bool,
    jitter_values: BTreeMap<String, bool>,
    jitter_configured: BTreeMap<String, bool>,
    io_scheduler: String,
    io_tone: String,
    wifi_no_restart: bool,
    auto_reapply: bool,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            policy: Settings::default(),
            policy_configured: false,
            bluetooth_hal: "offload".to_string(),
            bluetooth_hal_configured: false,
            resampler_preset: "179-408-99".to_string(),
            resampler_configured: false,
            resampler_bypass: "none".to_string(),
            resampler_cheat: true,
            resampler_stop_band: 179,
            resampler_half_length: 408,
            resampler_percent: 99,
            usb_period: 2_250,
            usb_period_configured: false,
            diagnostic: "audio".to_string(),
            diagnostic_all: false,
            jitter_values: JITTER_FEATURES
                .iter()
                .map(|feature| ((*feature).to_string(), false))
                .collect(),
            jitter_configured: JITTER_FEATURES
                .iter()
                .map(|feature| ((*feature).to_string(), false))
                .collect(),
            io_scheduler: "*".to_string(),
            io_tone: "medium".to_string(),
            wifi_no_restart: false,
            auto_reapply: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReapplyAction {
    Policy(Settings),
    Extra(Vec<String>),
}

#[derive(Clone, Debug)]
struct NamespaceInfo {
    self_ns: Option<String>,
    audio_ns: Option<String>,
    audio_pid: Option<u32>,
}

impl NamespaceInfo {
    fn matches(&self) -> Option<bool> {
        match (&self.self_ns, &self.audio_ns) {
            (Some(left), Some(right)) => Some(left == right),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Apply,
    Reset,
}

impl Action {
    fn script_name(self) -> &'static str {
        match self {
            Self::Apply => "apply.sh",
            Self::Reset => "reset.sh",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Reset => "reset",
        }
    }
}

struct OperationLock {
    path: PathBuf,
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn main() {
    restore_default_sigpipe();
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(unix)]
fn restore_default_sigpipe() {
    // Rust ignores SIGPIPE and turns a closed stdout pipe into a panic from
    // println!. CLI output is intentionally pipe-friendly (`schema | head`).
    unsafe extern "C" {
        fn signal(signal: i32, handler: usize) -> usize;
    }
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

fn run() -> Result<i32, String> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("schema") if args.len() == 2 => {
            print_schema(&module_dir()?);
            Ok(0)
        }
        Some("status") if args.len() == 2 => {
            print_status(&module_dir()?);
            Ok(0)
        }
        Some("logs") if args.len() == 2 => {
            print_logs()?;
            Ok(0)
        }
        Some("generated") if args.len() == 2 => {
            print_generated()?;
            Ok(0)
        }
        Some("aidl-inspect") if args.len() == 2 => {
            print_aidl_inspect()?;
            Ok(0)
        }
        Some("aidl-inspect-stdin") if args.len() == 2 => {
            let mut dump = String::new();
            std::io::stdin()
                .read_to_string(&mut dump)
                .map_err(|error| format!("cannot read audio policy dump: {error}"))?;
            print_aidl_snapshot(&aidl::parse_audio_policy_dump(&dump))?;
            Ok(0)
        }
        Some("aidl-preview") if args.len() == 5 => {
            let snapshot = audio_policy_snapshot()?;
            let source = discover_aidl_primary_config(&snapshot)?;
            print_aidl_plan_preview(&snapshot, &source, &args[2], &args[3], &args[4])?;
            Ok(0)
        }
        Some("aidl-match-stdin") if args.len() == 3 => {
            let mut dump = String::new();
            std::io::stdin()
                .read_to_string(&mut dump)
                .map_err(|error| format!("cannot read audio policy dump: {error}"))?;
            let xml = fs::read_to_string(&args[2])
                .map_err(|error| format!("cannot read {}: {error}", args[2]))?;
            let snapshot = aidl::parse_audio_policy_dump(&dump);
            let config = aidl::parse_module_xml(&xml)?;
            let runtime = snapshot
                .modules
                .iter()
                .find(|module| module.name == "primary" || module.name == "default")
                .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
            let (matched, total) = aidl::topology_match_score(runtime, &config);
            println!("runtime_module={}", runtime.name);
            println!("config_module={}", config.name);
            println!("topology_matches={matched}");
            println!("topology_total={total}");
            println!(
                "topology_match_percent={}",
                if total == 0 { 0 } else { matched * 100 / total }
            );
            Ok(0)
        }
        Some("aidl-rewrite-validate-stdin") if args.len() == 6 => {
            let mut dump = String::new();
            std::io::stdin()
                .read_to_string(&mut dump)
                .map_err(|error| format!("cannot read audio policy dump: {error}"))?;
            let source = fs::read_to_string(&args[2])
                .map_err(|error| format!("cannot read {}: {error}", args[2]))?;
            let rate = args[4]
                .parse::<u32>()
                .map_err(|_| format!("invalid sample rate: {}", args[4]))?;
            let pcm_type = match args[5].as_str() {
                "16" => "INT_16_BIT",
                "24" => "INT_24_BIT",
                "32" => "INT_32_BIT",
                "float" => "FLOAT_32_BIT",
                value => return Err(format!("unsupported bit depth: {value}")),
            };
            let snapshot = aidl::parse_audio_policy_dump(&dump);
            let runtime = snapshot
                .modules
                .iter()
                .find(|module| module.name == "primary" || module.name == "default")
                .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
            let source_model = aidl::parse_module_xml(&source)?;
            let (source_matches, source_total) = aidl::topology_match_score(runtime, &source_model);
            if source_total == 0 || source_matches != source_total {
                return Err(format!(
                    "source XML topology mismatch: {source_matches}/{source_total}"
                ));
            }
            let rewritten = aidl::rewrite_pcm_mix_port(&source, &args[3], rate, pcm_type)?;
            let rewritten_model = aidl::parse_module_xml(&rewritten)?;
            let (result_matches, result_total) =
                aidl::topology_match_score(runtime, &rewritten_model);
            if result_matches != result_total {
                return Err(format!(
                    "rewritten XML topology mismatch: {result_matches}/{result_total}"
                ));
            }
            let port = rewritten_model
                .output_mix_ports
                .iter()
                .find(|port| port.name == args[3])
                .ok_or_else(|| "rewritten mix port disappeared".to_string())?;
            println!("source_topology={source_matches}/{source_total}");
            println!("result_topology={result_matches}/{result_total}");
            println!("target_mix_port={}", port.name);
            println!("target_behavior={}", port.behavior());
            println!("target_rate={rate}");
            println!("target_pcm_type={pcm_type}");
            println!("source_bytes={}", source.len());
            println!("result_bytes={}", rewritten.len());
            Ok(0)
        }
        Some("aidl-plan-stdin") if args.len() == 6 || args.len() == 7 => {
            let mut dump = String::new();
            std::io::stdin()
                .read_to_string(&mut dump)
                .map_err(|error| format!("cannot read audio policy dump: {error}"))?;
            let rate = normalize_sample_rate(&args[4])?;
            let source = fs::read_to_string(&args[2])
                .map_err(|error| format!("cannot read {}: {error}", args[2]))?;
            let snapshot = aidl::parse_audio_policy_dump(&dump);
            let runtime = snapshot
                .modules
                .iter()
                .find(|module| module.name == "primary" || module.name == "default")
                .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
            let before = aidl::parse_module_xml(&source)?;
            let (source_matches, source_total) = aidl::topology_match_score(runtime, &before);
            if source_total == 0 || source_matches != source_total {
                return Err(format!(
                    "source XML topology mismatch: {source_matches}/{source_total}"
                ));
            }
            let plan = aidl::plan_policy(&snapshot, &args[3], rate, &args[5])?;
            let candidate = aidl::apply_plan_to_module_xml(&source, &before.name, &plan)?;
            let after = aidl::parse_module_xml(&candidate)?;
            let (result_matches, result_total) = aidl::topology_match_score(runtime, &after);
            if result_matches != result_total {
                return Err(format!(
                    "candidate XML topology mismatch: {result_matches}/{result_total}"
                ));
            }
            let structural = aidl::structural_diff(&before, &after);
            if !structural.is_empty() {
                return Err(format!(
                    "candidate changes module structure: {}",
                    structural.join("; ")
                ));
            }
            println!("policy={}", plan.policy);
            println!("runtime_module={}", runtime.name);
            println!("config_module={}", before.name);
            println!("source_topology={source_matches}/{source_total}");
            println!("candidate_topology={result_matches}/{result_total}");
            for target in &plan.targets {
                println!(
                    "target={}|{}|{}|{}|{}",
                    target.module,
                    target.port,
                    target.behavior,
                    target.rate,
                    target.pcm_type
                );
            }
            for (port, behavior, reason) in &plan.skipped {
                println!("skipped={port}|{behavior}|{reason}");
            }
            for change in aidl::profile_diff(&before, &after) {
                println!("profile_change={change}");
            }
            println!("structure_changes=0");
            if let Some(destination) = args.get(6) {
                atomic_write(Path::new(destination), candidate.as_bytes(), 0o644)?;
                println!("candidate_written={destination}");
            }
            println!("apply_enabled=0");
            Ok(0)
        }
        Some("preview") => {
            let settings = parse_settings_args(&args[2..])?;
            validate_settings(&settings, &module_dir()?)?;
            print!(
                "{}",
                render_script(&settings, &module_dir()?, Action::Apply)
            );
            Ok(0)
        }
        Some("apply") => {
            let settings = parse_settings_args(&args[2..])?;
            if audio_policy_snapshot().is_ok_and(|snapshot| snapshot.is_aidl()) {
                return run_aidl_operation(settings, Action::Apply);
            }
            run_operation(settings, Action::Apply)
        }
        Some("reset") if args.len() == 2 => {
            if audio_policy_snapshot().is_ok_and(|snapshot| snapshot.is_aidl()) {
                run_aidl_operation(Settings::default(), Action::Reset)
            } else {
                run_operation(Settings::default(), Action::Reset)
            }
        }
        Some("aidl-restore") if args.len() == 2 => run_aidl_restore(),
        Some("extra") => run_extra(&args[2..]),
        Some("settings") => run_settings_command(&args[2..]),
        Some("reapply") if args.len() == 2 => run_reapply(),
        _ => Err(format!(
            "usage: {} {{schema|status|logs|generated|aidl-inspect|aidl-inspect-stdin|aidl-preview POLICY RATE DEPTH|aidl-restore|aidl-match-stdin XML|aidl-rewrite-validate-stdin XML PORT RATE DEPTH|aidl-plan-stdin XML POLICY RATE DEPTH [OUT]|preview OPTIONS|apply OPTIONS|reset|extra TOOL ACTION|settings auto-reapply enable|disable|reapply}}",
            args.first().map(String::as_str).unwrap_or("usbsrctl")
        )),
    }
}

struct ExtraCommand {
    tool: String,
    script: &'static str,
    args: Vec<String>,
    reconnect_bluetooth: bool,
}

const RESAMPLER_PRESETS: &[(&str, &[&str])] = &[
    ("default", &[]),
    ("159-480-92", &["159", "480", "92"]),
    ("165-360-104", &["--bypass", "--cheat", "165", "360", "104"]),
    ("179-408-99", &["--cheat", "179", "408", "99"]),
    ("194-520-100", &["194", "520", "100"]),
    ("ultra-hifi", &["--cheat", "194", "520", "98"]),
    ("cheap-44", &["194", "520", "92"]),
    ("cheap-44-low", &["194", "520", "91"]),
    ("cheap-48", &["194", "520", "84"]),
    ("cheap-48-low", &["194", "520", "83"]),
    ("cheap-96", &["--bypass-hires", "194", "520", "42"]),
    ("mock-dac-a", &["--bypass", "--cheat", "150", "80", "109"]),
    ("mock-dac-b", &["120", "80", "97"]),
    ("mock-dac-c", &["--bypass", "--cheat", "100", "80", "104"]),
    ("mock-mastering", &["--cheat", "159", "240", "99"]),
];

fn run_extra(args: &[String]) -> Result<i32, String> {
    let command = parse_extra_command(args)?;
    let module_dir = module_dir()?;
    let script_path = module_dir
        .join(CORE_DIR)
        .join("extras")
        .join(command.script);
    if !script_path.is_file() {
        return Err(format!(
            "extras script not found: {}",
            script_path.display()
        ));
    }
    ensure_state_layout()?;
    let _lock = acquire_lock()?;
    let generated_path = state_root()
        .join("generated")
        .join(format!("extra-{}.sh", command.tool));
    let generated = render_extra_script(&script_path, &command.args);
    atomic_write(&generated_path, generated.as_bytes(), 0o700)?;

    let a2dp_was_connected = command.reconnect_bluetooth && bluetooth_a2dp_connected();
    let namespace = namespace_info();
    let (route, mut output) = execute_generated(&generated_path, &namespace)?;
    let upstream_code = output.status.code().unwrap_or(1);
    let mut code = upstream_code;
    if upstream_code == 0 && a2dp_was_connected {
        output
            .stdout
            .extend_from_slice(b"bluetooth_a2dp_before=1\n");
        match restore_bluetooth_a2dp() {
            Ok(recovery) => output.stdout.extend_from_slice(
                format!("bluetooth_recovery={recovery}\nbluetooth_a2dp_after=1\n").as_bytes(),
            ),
            Err(error) => {
                code = 72;
                output.stderr.extend_from_slice(
                    format!("Bluetooth A2DP recovery failed: {error}\n").as_bytes(),
                );
            }
        }
    }
    if upstream_code == 0 {
        persist_extra_settings(args)?;
    }
    let label = format!("extra-{}", command.tool);
    write_named_operation_log(&label, route, code, &generated_path, &output)?;
    println!("controller_action={label}");
    println!("controller_route={route}");
    println!("generated_script={}", generated_path.display());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(code)
}

fn parse_extra_command(args: &[String]) -> Result<ExtraCommand, String> {
    let tool = args.first().map(String::as_str).ok_or_else(extra_usage)?;
    let action = args.get(1).map(String::as_str).ok_or_else(extra_usage)?;
    match tool {
        "bluetooth-hal" => {
            if args.len() != 2
                || !matches!(action, "status" | "aosp" | "legacy" | "offload" | "sysbta")
            {
                return Err(
                    "bluetooth-hal action must be status, aosp, legacy, offload, or sysbta"
                        .to_string(),
                );
            }
            Ok(ExtraCommand {
                tool: tool.to_string(),
                script: "change-bluetooth-hal.sh",
                args: vec![if action == "status" {
                    "--status"
                } else {
                    action
                }
                .to_string()],
                reconnect_bluetooth: action != "status",
            })
        }
        "resampler" => {
            if action == "custom" {
                return parse_custom_resampler(args);
            }
            if args.len() != 2 {
                return Err(
                    "resampler action must be status, reset, a preset name, or custom parameters"
                        .to_string(),
                );
            }
            let mapped = match action {
                "status" => vec!["--status".to_string()],
                "reset" => vec!["--reset".to_string()],
                preset => RESAMPLER_PRESETS
                    .iter()
                    .find(|(name, _)| *name == preset)
                    .map(|(_, values)| values.iter().map(|value| (*value).to_string()).collect())
                    .ok_or_else(|| format!("unsupported resampler preset: {preset}"))?,
            };
            Ok(ExtraCommand {
                tool: tool.to_string(),
                script: "change-resampling-quality.sh",
                args: mapped,
                reconnect_bluetooth: action != "status",
            })
        }
        "usb-period" => {
            if args.len() != 2 {
                return Err(
                    "usb-period action must be status, reset, or a period in usec".to_string(),
                );
            }
            let value = match action {
                "status" => "--status".to_string(),
                "reset" => "--reset".to_string(),
                raw => {
                    let period = raw
                        .parse::<u32>()
                        .map_err(|_| format!("invalid USB period: {raw}"))?;
                    if !(125..=50_000).contains(&period) || period % 125 != 0 {
                        return Err(
                            "USB period must be a multiple of 125 between 125 and 50000 usec"
                                .to_string(),
                        );
                    }
                    period.to_string()
                }
            };
            Ok(ExtraCommand {
                tool: tool.to_string(),
                script: "change-usb-period.sh",
                args: vec![value],
                reconnect_bluetooth: action != "status",
            })
        }
        "jitter" => parse_jitter_command(args),
        "diagnose" => parse_diagnostic_command(args),
        _ => Err(extra_usage()),
    }
}

fn parse_jitter_command(args: &[String]) -> Result<ExtraCommand, String> {
    if args.get(1).map(String::as_str) == Some("status") && args.len() == 2 {
        return Ok(ExtraCommand {
            tool: "jitter".to_string(),
            script: "jitter-reducer.sh",
            args: vec!["--status".to_string()],
            reconnect_bluetooth: false,
        });
    }
    if args.len() < 3 || !matches!(args[1].as_str(), "enable" | "disable") {
        return Err(
            "jitter usage: jitter status | jitter enable|disable FEATURE [SCHEDULER TONE]"
                .to_string(),
        );
    }
    let enable = args[1] == "enable";
    let feature = args[2].as_str();
    let mut flag = match (enable, feature) {
        (true, "all") => "--all",
        (false, "all") => "++all",
        (true, "selinux") => "--selinux",
        (false, "selinux") => "++selinux",
        (true, "thermal") => "--thermal",
        (false, "thermal") => "++thermal",
        (true, "doze") => "--doze",
        (false, "doze") => "++doze",
        (true, "governor") => "--governor",
        (false, "governor") => "++governor",
        (true, "camera") => "--camera",
        (false, "camera") => "++camera",
        (true, "logd") => "--logd",
        (false, "logd") => "++logd",
        (true, "io") => "--io",
        (false, "io") => "++io",
        (true, "vm") => "--vm",
        (false, "vm") => "++vm",
        (true, "wifi") => "--wifi",
        (false, "wifi") => "++wifi",
        (true, "battery") => "--battery",
        (false, "battery") => "++battery",
        (true, "effect") => "--effect",
        (false, "effect") => "++effect",
        _ => return Err(format!("unsupported jitter feature: {feature}")),
    };
    if feature == "wifi" && enable && args.get(3).map(String::as_str) == Some("no-restart") {
        flag = "--wifi-no-restart";
    }
    let mut mapped = vec![flag.to_string()];
    if feature == "io" && enable {
        let scheduler = args.get(3).map(String::as_str).unwrap_or("*");
        let tone = args.get(4).map(String::as_str).unwrap_or("medium");
        if !IO_SCHEDULERS.contains(&scheduler) || !IO_TONES.contains(&tone) || args.len() > 5 {
            return Err("unsupported I/O scheduler or tone".to_string());
        }
        mapped.push(scheduler.to_string());
        mapped.push(tone.to_string());
    } else if !(args.len() == 3
        || (feature == "wifi" && enable && args.len() == 4 && args[3] == "no-restart"))
    {
        return Err("only enabled I/O accepts scheduler and tone arguments".to_string());
    }
    mapped.push("--status".to_string());
    Ok(ExtraCommand {
        tool: "jitter".to_string(),
        script: "jitter-reducer.sh",
        args: mapped,
        reconnect_bluetooth: false,
    })
}

fn parse_custom_resampler(args: &[String]) -> Result<ExtraCommand, String> {
    if args.len() != 7 {
        return Err("custom resampler usage: resampler custom none|48|96 cutoff|cheat STOP_DB HALF_LENGTH PERCENT".to_string());
    }
    let bypass = args[2].as_str();
    let mode = args[3].as_str();
    if !matches!(bypass, "none" | "48" | "96") || !matches!(mode, "cutoff" | "cheat") {
        return Err("invalid resampler bypass or cutoff mode".to_string());
    }
    let stop = args[4]
        .parse::<u32>()
        .map_err(|_| "invalid stop-band value".to_string())?;
    let half = args[5]
        .parse::<u32>()
        .map_err(|_| "invalid half-filter length".to_string())?;
    let percent = args[6]
        .parse::<u32>()
        .map_err(|_| "invalid cutoff/cheat percent".to_string())?;
    if !(20..=242).contains(&stop)
        || !(8..=640).contains(&half)
        || half % 8 != 0
        || (mode == "cutoff" && !(1..=100).contains(&percent))
        || (mode == "cheat" && !(1..=200).contains(&percent))
    {
        return Err("custom resampler parameters are outside upstream ranges".to_string());
    }
    let mut mapped = Vec::new();
    match bypass {
        "48" => mapped.push("--bypass".to_string()),
        "96" => mapped.push("--bypass-hires".to_string()),
        _ => {}
    }
    if mode == "cheat" {
        mapped.push("--cheat".to_string());
    }
    mapped.extend([stop.to_string(), half.to_string(), percent.to_string()]);
    Ok(ExtraCommand {
        tool: "resampler".to_string(),
        script: "change-resampling-quality.sh",
        args: mapped,
        reconnect_bluetooth: true,
    })
}

fn parse_diagnostic_command(args: &[String]) -> Result<ExtraCommand, String> {
    if !(2..=3).contains(&args.len()) {
        return Err("diagnose usage: diagnose audio|bluetooth|config|alsa [all]".to_string());
    }
    let script = match args[1].as_str() {
        "audio" => "dumpsys-filtered.sh",
        "bluetooth" => "dumpsys-bluetooth-filtered.sh",
        "config" => "getConfig.sh",
        "alsa" => "alsa-hw-params.sh",
        other => return Err(format!("unsupported diagnostic: {other}")),
    };
    let mapped = if args.get(2).map(String::as_str) == Some("all") {
        vec!["--all".to_string()]
    } else if args.len() == 2 {
        Vec::new()
    } else {
        return Err("diagnostic detail must be all".to_string());
    };
    Ok(ExtraCommand {
        tool: format!("diagnose-{}", args[1]),
        script,
        args: mapped,
        reconnect_bluetooth: false,
    })
}

fn extra_usage() -> String {
    "extra tool must be bluetooth-hal, resampler, usb-period, jitter, or diagnose".to_string()
}

fn render_extra_script(script: &Path, args: &[String]) -> String {
    let command = std::iter::once("/system/bin/sh".to_string())
        .chain(std::iter::once(script.to_string_lossy().into_owned()))
        .chain(args.iter().cloned())
        .map(|argument| shell_quote(&argument))
        .collect::<Vec<_>>()
        .join(" ");
    format!("#!/system/bin/sh\nset -u\nexec {command}\n")
}

fn module_dir() -> Result<PathBuf, String> {
    let executable =
        env::current_exe().map_err(|error| format!("cannot resolve executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve module directory".to_string())
}

fn state_root() -> PathBuf {
    PathBuf::from(STATE_ROOT)
}

fn ensure_state_layout() -> Result<(), String> {
    let root = state_root();
    let generated = root.join("generated");
    fs::create_dir_all(&generated)
        .map_err(|error| format!("cannot create state directory: {error}"))?;
    set_mode(&root, 0o700)?;
    set_mode(&generated, 0o700)?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("cannot set permissions on {}: {error}", path.display()))
}

fn parse_settings_args(args: &[String]) -> Result<Settings, String> {
    let mut settings = Settings::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--policy" => {
                settings.policy = take_value(args, &mut index, "--policy")?.to_string();
            }
            "--sample-rate" => {
                let value = take_value(args, &mut index, "--sample-rate")?;
                settings.sample_rate = normalize_sample_rate(value)?;
            }
            "--bit-depth" => {
                settings.bit_depth = take_value(args, &mut index, "--bit-depth")?.to_string();
            }
            "--drc" => settings.drc = true,
            "--force-usbv2" => settings.force_usbv2 = true,
            "--force-bluetooth-qti" => settings.force_bluetooth_qti = true,
            "--amzm" => settings.amzm = true,
            "--test" => settings.test = true,
            "--test-template" => {
                settings.test_template =
                    Some(take_value(args, &mut index, "--test-template")?.to_string());
            }
            unknown => return Err(format!("unknown controller option: {unknown}")),
        }
        index += 1;
    }
    Ok(settings)
}

fn take_value<'a>(args: &'a [String], index: &mut usize, option: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn normalize_sample_rate(value: &str) -> Result<u32, String> {
    let normalized = match value {
        "44k" | "44.1k" => 44_100,
        "48k" => 48_000,
        "88k" | "88.2k" => 88_200,
        "96k" => 96_000,
        "176k" | "176.4k" => 176_400,
        "192k" => 192_000,
        "352k" | "353k" | "352.8k" => 352_800,
        "384k" => 384_000,
        "705k" | "706k" | "705.6k" => 705_600,
        "768k" => 768_000,
        other => other
            .parse::<u32>()
            .map_err(|_| format!("invalid sample rate: {other}"))?,
    };
    if !(44_100..=768_000).contains(&normalized) {
        return Err(format!(
            "sample rate must be between 44100 and 768000 Hz: {normalized}"
        ));
    }
    Ok(normalized)
}

fn validate_settings(settings: &Settings, module_dir: &Path) -> Result<(), String> {
    if policy_flag(&settings.policy).is_none() {
        return Err(format!("unsupported policy mode: {}", settings.policy));
    }
    normalize_sample_rate(&settings.sample_rate.to_string())?;
    if !BIT_DEPTHS
        .iter()
        .any(|(value, _)| *value == settings.bit_depth)
    {
        return Err(format!("unsupported bit depth: {}", settings.bit_depth));
    }
    if settings.amzm && settings.test {
        return Err("--amzm and --test are mutually exclusive in the WebUI".to_string());
    }
    match (&settings.test, &settings.test_template) {
        (true, Some(template)) => validate_template(template, module_dir)?,
        (true, None) => {
            return Err(
                "--test requires a template because the upstream default is absent".to_string(),
            )
        }
        (false, Some(_)) => return Err("--test-template requires --test".to_string()),
        (false, None) => {}
    }
    let script = module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh");
    if !script.is_file() {
        return Err(format!("upstream script not found: {}", script.display()));
    }
    Ok(())
}

fn validate_template(template: &str, module_dir: &Path) -> Result<(), String> {
    if template.is_empty()
        || template.starts_with('/')
        || template
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !template.ends_with(".xml")
        || !template
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./-".contains(&byte))
    {
        return Err(format!("unsafe template path: {template}"));
    }
    let path = module_dir.join(CORE_DIR).join("templates").join(template);
    if !path.is_file() {
        return Err(format!("template does not exist: {template}"));
    }
    Ok(())
}

fn policy_flag(policy: &str) -> Option<&'static str> {
    POLICIES
        .iter()
        .find(|(value, _, _)| *value == policy)
        .map(|(_, flag, _)| *flag)
}

fn upstream_args(settings: &Settings, action: Action) -> Vec<String> {
    if matches!(action, Action::Reset) {
        return vec!["--reset".to_string()];
    }

    let mut args = vec![policy_flag(&settings.policy)
        .unwrap_or("--auto")
        .to_string()];
    if settings.drc {
        args.push("--drc".to_string());
    }
    if settings.force_usbv2 {
        args.push("--force-usbv2".to_string());
    }
    if settings.force_bluetooth_qti {
        args.push("--force-bluetooth-qti".to_string());
    }
    if settings.amzm {
        args.push("--amzm".to_string());
    }
    if settings.test {
        args.push("--test".to_string());
        if let Some(template) = &settings.test_template {
            args.push("--test-template".to_string());
            args.push(template.clone());
        }
    }
    args.push(settings.sample_rate.to_string());
    args.push(settings.bit_depth.clone());
    args
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn render_script(settings: &Settings, module_dir: &Path, action: Action) -> String {
    let script = module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh");
    let command = std::iter::once("/system/bin/sh".to_string())
        .chain(std::iter::once(script.to_string_lossy().into_owned()))
        .chain(upstream_args(settings, action))
        .map(|argument| shell_quote(&argument))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "#!/system/bin/sh\n\
set -u\n\
audio_pid=\"$(pidof audioserver 2>/dev/null)\"\n\
audio_pid=\"${{audio_pid%% *}}\"\n\
if [ -z \"$audio_pid\" ]; then\n\
    audio_pid=\"$(getprop init.svc_debug_pid.audioserver 2>/dev/null)\"\n\
fi\n\
[ -n \"$audio_pid\" ] || {{ echo \"audioserver is not running\" >&2; exit 70; }}\n\
self_ns=\"$(readlink /proc/self/ns/mnt 2>/dev/null)\"\n\
audio_ns=\"$(readlink \"/proc/$audio_pid/ns/mnt\" 2>/dev/null)\"\n\
if [ -z \"$self_ns\" ] || [ -z \"$audio_ns\" ] || [ \"$self_ns\" != \"$audio_ns\" ]; then\n\
    echo \"mount namespace mismatch: self=$self_ns audio=$audio_ns\" >&2\n\
    exit 71\n\
fi\n\
echo \"namespace verified: $self_ns\"\n\
exec {command}\n"
    )
}

fn run_operation(settings: Settings, action: Action) -> Result<i32, String> {
    let module_dir = module_dir()?;
    validate_settings_for_action(&settings, &module_dir, action)?;
    ensure_state_layout()?;
    let _lock = acquire_lock()?;

    let generated_path = state_root().join("generated").join(action.script_name());
    let generated = render_script(&settings, &module_dir, action);
    atomic_write(&generated_path, generated.as_bytes(), 0o700)?;

    let a2dp_was_connected = bluetooth_a2dp_connected();

    let namespace = namespace_info();
    let (route, mut output) = execute_generated(&generated_path, &namespace)?;
    let upstream_code = output.status.code().unwrap_or(1);
    let mut code = upstream_code;

    if upstream_code == 0 && a2dp_was_connected {
        output
            .stdout
            .extend_from_slice(b"bluetooth_a2dp_before=1\n");
        match restore_bluetooth_a2dp() {
            Ok(route) => {
                output.stdout.extend_from_slice(
                    format!("bluetooth_recovery={route}\nbluetooth_a2dp_after=1\n").as_bytes(),
                );
            }
            Err(error) => {
                code = 72;
                output.stderr.extend_from_slice(
                    format!("Bluetooth A2DP recovery failed: {error}\n").as_bytes(),
                );
            }
        }
    }

    write_operation_log(action, route, code, &generated_path, &output)?;
    if upstream_code == 0 {
        match action {
            Action::Apply => save_policy_settings(&settings)?,
            Action::Reset => reset_policy_settings()?,
        }
    }

    println!("controller_action={}", action.label());
    println!("controller_route={route}");
    println!("generated_script={}", generated_path.display());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(code)
}

fn bluetooth_a2dp_connected() -> bool {
    let Ok(output) = Command::new("dumpsys").arg("audio").output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    bluetooth_a2dp_connected_in_dump(&String::from_utf8_lossy(&output.stdout))
}

fn bluetooth_a2dp_connected_in_dump(dump: &str) -> bool {
    let Some((_, connected_and_rest)) = dump.split_once("Connected devices:") else {
        return false;
    };
    let connected = connected_and_rest
        .split_once("APM Connected device")
        .map(|(section, _)| section)
        .unwrap_or(connected_and_rest);
    if !connected.contains("(bt_a2dp)") {
        return false;
    }

    let Some((_, music_and_rest)) = dump.split_once("- STREAM_MUSIC:") else {
        return false;
    };
    let music = music_and_rest
        .split_once("\n- STREAM_")
        .map(|(section, _)| section)
        .unwrap_or(music_and_rest);
    music
        .lines()
        .any(|line| line.trim_start().starts_with("Devices:") && line.contains("bt_a2dp"))
}

fn wait_for_bluetooth_a2dp(attempts: usize, delay: Duration) -> bool {
    for _ in 0..attempts {
        if bluetooth_a2dp_connected() {
            return true;
        }
        std::thread::sleep(delay);
    }
    false
}

fn restore_bluetooth_a2dp() -> Result<&'static str, String> {
    // AudioService can retain the old device list briefly after audioserver has
    // restarted. Let the new policy instance settle before deciding that A2DP
    // recovered without intervention.
    std::thread::sleep(Duration::from_secs(3));
    if wait_for_bluetooth_a2dp(8, Duration::from_millis(500)) {
        return Ok("automatic");
    }
    Err(
        "audio policy was applied, but the connected headset must be explicitly disconnected and reconnected before STREAM_MUSIC returns to A2DP"
            .to_string(),
    )
}

fn validate_settings_for_action(
    settings: &Settings,
    module_dir: &Path,
    action: Action,
) -> Result<(), String> {
    match action {
        Action::Apply => validate_settings(settings, module_dir),
        Action::Reset => {
            let script = module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh");
            if script.is_file() {
                Ok(())
            } else {
                Err(format!("upstream script not found: {}", script.display()))
            }
        }
    }
}

fn acquire_lock() -> Result<OperationLock, String> {
    let path = state_root().join("operation.lock");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", std::process::id());
            Ok(OperationLock { path })
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let stale = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                .map(|elapsed| elapsed > Duration::from_secs(180))
                .unwrap_or(false);
            if stale {
                fs::remove_file(&path).map_err(|remove_error| {
                    format!("cannot remove stale operation lock: {remove_error}")
                })?;
                acquire_lock()
            } else {
                Err("another audio operation is still running".to_string())
            }
        }
        Err(error) => Err(format!("cannot acquire operation lock: {error}")),
    }
}

fn execute_generated(
    path: &Path,
    namespace: &NamespaceInfo,
) -> Result<(&'static str, Output), String> {
    if namespace.matches() == Some(true) {
        let output = Command::new("/system/bin/sh")
            .arg(path)
            .output()
            .map_err(|error| format!("cannot execute generated script: {error}"))?;
        return Ok(("current-namespace", output));
    }

    let command = format!("/system/bin/sh {}", shell_quote(&path.to_string_lossy()));
    let output = Command::new("su")
        .args(["--mount-master", "-c", command.as_str()])
        .output()
        .map_err(|error| format!("cannot execute mount-master shell: {error}"))?;
    Ok(("mount-master", output))
}

fn atomic_write(path: &Path, content: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid output path: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(OsStr::to_str).unwrap_or("output"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .map_err(|error| format!("cannot create {}: {error}", temp.display()))?;
    if let Err(error) = file.write_all(content).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot write {}: {error}", temp.display()));
    }
    set_mode(&temp, mode)?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("cannot replace {}: {error}", path.display())
    })
}

fn render_stored_settings(settings: &StoredSettings) -> String {
    let mut content = format!(
        concat!(
            "version={}\n",
            "policy_configured={}\npolicy={}\nsample_rate={}\nbit_depth={}\n",
            "drc={}\nforce_usbv2={}\nforce_bluetooth_qti={}\namzm={}\ntest={}\ntest_template={}\n",
            "bluetooth_hal_configured={}\nbluetooth_hal={}\n",
            "resampler_configured={}\nresampler_preset={}\nresampler_bypass={}\n",
            "resampler_cheat={}\nresampler_stop_band={}\nresampler_half_length={}\nresampler_percent={}\n",
            "usb_period_configured={}\nusb_period={}\n",
            "diagnostic={}\ndiagnostic_all={}\n",
            "io_scheduler={}\nio_tone={}\nwifi_no_restart={}\nauto_reapply={}\n"
        ),
        SETTINGS_VERSION,
        bool_number(settings.policy_configured),
        settings.policy.policy,
        settings.policy.sample_rate,
        settings.policy.bit_depth,
        bool_number(settings.policy.drc),
        bool_number(settings.policy.force_usbv2),
        bool_number(settings.policy.force_bluetooth_qti),
        bool_number(settings.policy.amzm),
        bool_number(settings.policy.test),
        settings.policy.test_template.as_deref().unwrap_or(""),
        bool_number(settings.bluetooth_hal_configured),
        settings.bluetooth_hal,
        bool_number(settings.resampler_configured),
        settings.resampler_preset,
        settings.resampler_bypass,
        bool_number(settings.resampler_cheat),
        settings.resampler_stop_band,
        settings.resampler_half_length,
        settings.resampler_percent,
        bool_number(settings.usb_period_configured),
        settings.usb_period,
        settings.diagnostic,
        bool_number(settings.diagnostic_all),
        settings.io_scheduler,
        settings.io_tone,
        bool_number(settings.wifi_no_restart),
        bool_number(settings.auto_reapply),
    );
    for feature in JITTER_FEATURES {
        content.push_str(&format!(
            "jitter_{feature}={}\njitter_{feature}_configured={}\n",
            bool_number(
                settings
                    .jitter_values
                    .get(*feature)
                    .copied()
                    .unwrap_or(false)
            ),
            bool_number(
                settings
                    .jitter_configured
                    .get(*feature)
                    .copied()
                    .unwrap_or(false)
            )
        ));
    }
    content
}

fn parse_stored_settings(content: &str) -> StoredSettings {
    let mut settings = StoredSettings::default();
    settings.policy_configured = !content.trim().is_empty()
        && !content
            .lines()
            .any(|line| line.starts_with("policy_configured="));
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "policy_configured" => settings.policy_configured = value == "1",
            "policy" if POLICIES.iter().any(|(name, _, _)| *name == value) => {
                settings.policy.policy = value.to_string()
            }
            "sample_rate" => {
                if let Ok(rate) = normalize_sample_rate(value) {
                    settings.policy.sample_rate = rate;
                }
            }
            "bit_depth" if BIT_DEPTHS.iter().any(|(depth, _)| *depth == value) => {
                settings.policy.bit_depth = value.to_string()
            }
            "drc" => settings.policy.drc = value == "1",
            "force_usbv2" => settings.policy.force_usbv2 = value == "1",
            "force_bluetooth_qti" => settings.policy.force_bluetooth_qti = value == "1",
            "amzm" => settings.policy.amzm = value == "1",
            "test" => settings.policy.test = value == "1",
            "test_template" if !value.is_empty() => {
                settings.policy.test_template = Some(value.to_string())
            }
            "bluetooth_hal_configured" => settings.bluetooth_hal_configured = value == "1",
            "bluetooth_hal" if matches!(value, "aosp" | "legacy" | "offload" | "sysbta") => {
                settings.bluetooth_hal = value.to_string()
            }
            "resampler_configured" => settings.resampler_configured = value == "1",
            "resampler_preset"
                if value == "custom"
                    || RESAMPLER_PRESETS.iter().any(|(name, _)| *name == value) =>
            {
                settings.resampler_preset = value.to_string()
            }
            "resampler_bypass" if matches!(value, "none" | "48" | "96") => {
                settings.resampler_bypass = value.to_string()
            }
            "resampler_cheat" => settings.resampler_cheat = value == "1",
            "resampler_stop_band" => {
                if let Ok(value) = value.parse::<u32>() {
                    if (20..=242).contains(&value) {
                        settings.resampler_stop_band = value;
                    }
                }
            }
            "resampler_half_length" => {
                if let Ok(value) = value.parse::<u32>() {
                    if (8..=640).contains(&value) && value % 8 == 0 {
                        settings.resampler_half_length = value;
                    }
                }
            }
            "resampler_percent" => {
                if let Ok(value) = value.parse::<u32>() {
                    if (1..=200).contains(&value) {
                        settings.resampler_percent = value;
                    }
                }
            }
            "usb_period_configured" => settings.usb_period_configured = value == "1",
            "usb_period" => {
                if let Ok(value) = value.parse::<u32>() {
                    if (125..=50_000).contains(&value) && value % 125 == 0 {
                        settings.usb_period = value;
                    }
                }
            }
            "diagnostic" if matches!(value, "audio" | "bluetooth" | "config" | "alsa") => {
                settings.diagnostic = value.to_string()
            }
            "diagnostic_all" => settings.diagnostic_all = value == "1",
            "io_scheduler" if IO_SCHEDULERS.contains(&value) => {
                settings.io_scheduler = value.to_string()
            }
            "io_tone" if IO_TONES.contains(&value) => settings.io_tone = value.to_string(),
            "wifi_no_restart" => settings.wifi_no_restart = value == "1",
            "auto_reapply" => settings.auto_reapply = value == "1",
            _ => {}
        }
        if let Some(feature) = key
            .strip_prefix("jitter_")
            .and_then(|key| key.strip_suffix("_configured"))
            .filter(|feature| JITTER_FEATURES.contains(feature))
        {
            settings
                .jitter_configured
                .insert(feature.to_string(), value == "1");
        } else if let Some(feature) = key
            .strip_prefix("jitter_")
            .filter(|feature| JITTER_FEATURES.contains(feature))
        {
            settings
                .jitter_values
                .insert(feature.to_string(), value == "1");
        }
    }
    if !settings.resampler_cheat && settings.resampler_percent > 100 {
        settings.resampler_percent = 100;
    }
    settings
}

fn load_stored_settings() -> StoredSettings {
    let mut settings = fs::read_to_string(state_root().join("settings.conf"))
        .map(|content| parse_stored_settings(&content))
        .unwrap_or_default();
    if settings.resampler_configured && settings.resampler_preset != "custom" {
        let preset = settings.resampler_preset.clone();
        store_resampler_preset(&mut settings, &preset);
    }
    settings
}

fn save_stored_settings(settings: &StoredSettings) -> Result<(), String> {
    atomic_write(
        &state_root().join("settings.conf"),
        render_stored_settings(settings).as_bytes(),
        0o600,
    )
}

fn save_policy_settings(policy: &Settings) -> Result<(), String> {
    let mut settings = load_stored_settings();
    settings.policy = policy.clone();
    settings.policy_configured = true;
    save_stored_settings(&settings)
}

fn reset_policy_settings() -> Result<(), String> {
    let mut settings = load_stored_settings();
    settings.policy = Settings::default();
    settings.policy_configured = false;
    save_stored_settings(&settings)
}

fn persist_extra_settings(args: &[String]) -> Result<(), String> {
    let mut settings = load_stored_settings();
    if update_stored_settings_for_extra(&mut settings, args) {
        save_stored_settings(&settings)?;
    }
    Ok(())
}

fn store_resampler_preset(settings: &mut StoredSettings, preset: &str) {
    let Some((_, values)) = RESAMPLER_PRESETS.iter().find(|(name, _)| *name == preset) else {
        return;
    };
    settings.resampler_preset = preset.to_string();
    settings.resampler_bypass = if values.contains(&"--bypass-hires") {
        "96"
    } else if values.contains(&"--bypass") {
        "48"
    } else {
        "none"
    }
    .to_string();
    settings.resampler_cheat = values.contains(&"--cheat") || values.is_empty();
    let numbers = values
        .iter()
        .filter_map(|value| value.parse::<u32>().ok())
        .collect::<Vec<_>>();
    if let [stop_band, half_length, percent] = numbers.as_slice() {
        settings.resampler_stop_band = *stop_band;
        settings.resampler_half_length = *half_length;
        settings.resampler_percent = *percent;
    } else if values.is_empty() {
        let defaults = StoredSettings::default();
        settings.resampler_stop_band = defaults.resampler_stop_band;
        settings.resampler_half_length = defaults.resampler_half_length;
        settings.resampler_percent = defaults.resampler_percent;
    }
    settings.resampler_configured = true;
}

fn update_stored_settings_for_extra(settings: &mut StoredSettings, args: &[String]) -> bool {
    let Some(tool) = args.first().map(String::as_str) else {
        return false;
    };
    match tool {
        "bluetooth-hal" => match args.get(1).map(String::as_str) {
            Some("status") | None => false,
            Some(value) => {
                settings.bluetooth_hal = value.to_string();
                settings.bluetooth_hal_configured = true;
                true
            }
        },
        "resampler" => match args.get(1).map(String::as_str) {
            Some("status") | None => false,
            Some("reset") => {
                let defaults = StoredSettings::default();
                settings.resampler_preset = defaults.resampler_preset;
                settings.resampler_bypass = defaults.resampler_bypass;
                settings.resampler_cheat = defaults.resampler_cheat;
                settings.resampler_stop_band = defaults.resampler_stop_band;
                settings.resampler_half_length = defaults.resampler_half_length;
                settings.resampler_percent = defaults.resampler_percent;
                settings.resampler_configured = false;
                true
            }
            Some("custom") => {
                settings.resampler_preset = "custom".to_string();
                settings.resampler_bypass = args[2].clone();
                settings.resampler_cheat = args[3] == "cheat";
                settings.resampler_stop_band = args[4].parse().unwrap_or(179);
                settings.resampler_half_length = args[5].parse().unwrap_or(408);
                settings.resampler_percent = args[6].parse().unwrap_or(99);
                settings.resampler_configured = true;
                true
            }
            Some(preset) => {
                store_resampler_preset(settings, preset);
                true
            }
        },
        "usb-period" => match args.get(1).map(String::as_str) {
            Some("status") | None => false,
            Some("reset") => {
                settings.usb_period = StoredSettings::default().usb_period;
                settings.usb_period_configured = false;
                true
            }
            Some(period) => {
                settings.usb_period = period.parse().unwrap_or(2_250);
                settings.usb_period_configured = true;
                true
            }
        },
        "jitter" => {
            if args.get(1).map(String::as_str) == Some("status") {
                return false;
            }
            let enabled = args.get(1).map(String::as_str) == Some("enable");
            let Some(feature) = args.get(2).map(String::as_str) else {
                return false;
            };
            if feature == "all" {
                for feature in JITTER_BASE_FEATURES {
                    settings
                        .jitter_values
                        .insert((*feature).to_string(), enabled);
                    settings
                        .jitter_configured
                        .insert((*feature).to_string(), enabled);
                }
                return true;
            }
            settings.jitter_values.insert(feature.to_string(), enabled);
            settings.jitter_configured.insert(feature.to_string(), true);
            if feature == "io" && enabled {
                settings.io_scheduler = args.get(3).cloned().unwrap_or_else(|| "*".to_string());
                settings.io_tone = args.get(4).cloned().unwrap_or_else(|| "medium".to_string());
            }
            if feature == "wifi" {
                settings.wifi_no_restart =
                    enabled && args.get(3).map(String::as_str) == Some("no-restart");
            }
            true
        }
        "diagnose" => {
            settings.diagnostic = args[1].clone();
            settings.diagnostic_all = args.get(2).map(String::as_str) == Some("all");
            true
        }
        _ => false,
    }
}

fn run_settings_command(args: &[String]) -> Result<i32, String> {
    if args.len() != 2 || args[0] != "auto-reapply" {
        return Err("settings usage: settings auto-reapply enable|disable".to_string());
    }
    let enabled = match args[1].as_str() {
        "enable" => true,
        "disable" => false,
        _ => return Err("auto-reapply must be enable or disable".to_string()),
    };
    ensure_state_layout()?;
    let mut settings = load_stored_settings();
    settings.auto_reapply = enabled;
    save_stored_settings(&settings)?;
    println!("auto_reapply={}", bool_number(enabled));
    Ok(0)
}

fn run_reapply() -> Result<i32, String> {
    let stored = load_stored_settings();
    if !stored.auto_reapply {
        println!("auto_reapply=0");
        return Ok(0);
    }
    let plan = build_reapply_plan(&stored);
    let aidl_policy_restored = read_aidl_target().is_ok_and(|target| path_is_mounted(&target));
    let mut applied = 0;
    for action in plan {
        let code = match action {
            ReapplyAction::Policy(_) if aidl_policy_restored => {
                println!("aidl_policy_reapply=already-restored");
                continue;
            }
            ReapplyAction::Policy(settings) => {
                if audio_policy_snapshot().is_ok_and(|snapshot| snapshot.is_aidl()) {
                    run_aidl_operation(settings, Action::Apply)?
                } else {
                    run_operation(settings, Action::Apply)?
                }
            }
            ReapplyAction::Extra(args) => run_extra(&args)?,
        };
        if code != 0 {
            return Ok(code);
        }
        applied += 1;
    }
    println!("reapplied={applied}");
    Ok(0)
}

fn build_reapply_plan(stored: &StoredSettings) -> Vec<ReapplyAction> {
    if !stored.auto_reapply {
        return Vec::new();
    }
    let mut plan = Vec::new();
    if stored.bluetooth_hal_configured {
        plan.push(ReapplyAction::Extra(vec![
            "bluetooth-hal".to_string(),
            stored.bluetooth_hal.clone(),
        ]));
    }
    if stored.policy_configured {
        plan.push(ReapplyAction::Policy(stored.policy.clone()));
    }
    if stored.resampler_configured {
        let args = if stored.resampler_preset == "custom" {
            vec![
                "resampler".to_string(),
                "custom".to_string(),
                stored.resampler_bypass.clone(),
                if stored.resampler_cheat {
                    "cheat"
                } else {
                    "cutoff"
                }
                .to_string(),
                stored.resampler_stop_band.to_string(),
                stored.resampler_half_length.to_string(),
                stored.resampler_percent.to_string(),
            ]
        } else {
            vec!["resampler".to_string(), stored.resampler_preset.clone()]
        };
        plan.push(ReapplyAction::Extra(args));
    }
    if stored.usb_period_configured {
        plan.push(ReapplyAction::Extra(vec![
            "usb-period".to_string(),
            stored.usb_period.to_string(),
        ]));
    }
    for feature in JITTER_FEATURES {
        if !stored
            .jitter_configured
            .get(*feature)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        let enabled = stored.jitter_values.get(*feature).copied().unwrap_or(false);
        let mut args = vec![
            "jitter".to_string(),
            if enabled { "enable" } else { "disable" }.to_string(),
            (*feature).to_string(),
        ];
        if *feature == "io" && enabled {
            args.push(stored.io_scheduler.clone());
            args.push(stored.io_tone.clone());
        } else if *feature == "wifi" && enabled && stored.wifi_no_restart {
            args.push("no-restart".to_string());
        }
        plan.push(ReapplyAction::Extra(args));
    }
    plan
}

fn bool_number(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

fn write_operation_log(
    action: Action,
    route: &str,
    code: i32,
    generated_path: &Path,
    output: &Output,
) -> Result<(), String> {
    write_named_operation_log(action.label(), route, code, generated_path, output)
}

fn write_named_operation_log(
    action: &str,
    route: &str,
    code: i32,
    generated_path: &Path,
    output: &Output,
) -> Result<(), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let status = format!(
        "last_action={}\nlast_route={route}\nlast_exit={code}\nlast_time={timestamp}\n",
        action
    );
    atomic_write(&state_root().join("last.status"), status.as_bytes(), 0o600)?;

    let log = format!(
        "time={timestamp}\naction={}\nroute={route}\nexit={code}\ngenerated={}\n\n[stdout]\n{}\n\n[stderr]\n{}\n",
        action,
        generated_path.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    atomic_write(&state_root().join("last.log"), log.as_bytes(), 0o600)
}

fn print_status(module_dir: &Path) {
    let stored = load_stored_settings();
    let settings = stored.policy.clone();
    let namespace = namespace_info();
    println!("controller_version={CONTROLLER_VERSION}");
    println!("script_version={}", upstream_script_version(module_dir));
    println!("module_dir={}", module_dir.display());
    print_settings(&settings);
    print_stored_settings(&stored);
    println!(
        "audioserver_pid={}",
        namespace
            .audio_pid
            .map(|pid| pid.to_string())
            .unwrap_or_default()
    );
    println!("self_ns={}", namespace.self_ns.as_deref().unwrap_or(""));
    println!("audio_ns={}", namespace.audio_ns.as_deref().unwrap_or(""));
    println!(
        "namespace_ok={}",
        match namespace.matches() {
            Some(true) => "1",
            Some(false) => "0",
            None => "unknown",
        }
    );
    print_audio_backend_status();
    if let Ok(last_status) = fs::read_to_string(state_root().join("last.status")) {
        print!("{last_status}");
    }
    print_templates(module_dir);
}

fn audio_policy_snapshot() -> Result<aidl::AudioPolicySnapshot, String> {
    let output = Command::new("dumpsys")
        .arg("media.audio_policy")
        .output()
        .map_err(|error| format!("cannot run media.audio_policy dump: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "media.audio_policy dump failed with status {}",
            output.status
        ));
    }
    Ok(aidl::parse_audio_policy_dump(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

#[derive(Clone, Debug)]
struct AidlConfigSource {
    path: PathBuf,
    xml: String,
    model: aidl::AudioModule,
    matches: usize,
    total: usize,
}

fn discover_aidl_primary_config(
    snapshot: &aidl::AudioPolicySnapshot,
) -> Result<AidlConfigSource, String> {
    if !snapshot.is_aidl() {
        return Err(format!(
            "active audio policy is not AIDL: {}",
            snapshot.config_source
        ));
    }
    let runtime = snapshot
        .modules
        .iter()
        .find(|module| module.name == "primary" || module.name == "default")
        .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
    let mut candidates = BTreeSet::new();
    for root in [
        "/odm/etc/audio",
        "/vendor/etc/audio",
        "/product/etc/audio",
        "/system_ext/etc/audio",
        "/system/etc/audio",
    ] {
        collect_named_files(
            Path::new(root),
            "audio_module_config_primary.xml",
            3,
            &mut candidates,
        );
    }
    let mut matches = Vec::new();
    for path in candidates {
        let Ok(xml) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(model) = aidl::parse_module_xml(&xml) else {
            continue;
        };
        let (matched, total) = aidl::topology_match_score(runtime, &model);
        if total > 0 && matched == total {
            matches.push(AidlConfigSource {
                path,
                xml,
                model,
                matches: matched,
                total,
            });
        }
    }
    match matches.len() {
        0 => Err("no audio_module_config_primary.xml fully matches the live AIDL topology"
            .to_string()),
        1 => Ok(matches.remove(0)),
        count => Err(format!(
            "{count} audio_module_config_primary.xml files fully match the live AIDL topology; refusing an ambiguous writer"
        )),
    }
}

fn collect_named_files(root: &Path, name: &str, depth: usize, output: &mut BTreeSet<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && entry.file_name() == OsStr::new(name) {
            output.insert(path);
        } else if file_type.is_dir() {
            collect_named_files(&path, name, depth - 1, output);
        }
    }
}

fn print_aidl_plan_preview(
    snapshot: &aidl::AudioPolicySnapshot,
    source: &AidlConfigSource,
    policy: &str,
    rate: &str,
    bit_depth: &str,
) -> Result<(), String> {
    let rate = normalize_sample_rate(rate)?;
    let runtime = snapshot
        .modules
        .iter()
        .find(|module| module.name == "primary" || module.name == "default")
        .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
    let plan = aidl::plan_policy(snapshot, policy, rate, bit_depth)?;
    let candidate = aidl::apply_plan_to_module_xml(&source.xml, &source.model.name, &plan)?;
    let after = aidl::parse_module_xml(&candidate)?;
    let (candidate_matches, candidate_total) = aidl::topology_match_score(runtime, &after);
    if candidate_total == 0 || candidate_matches != candidate_total {
        return Err(format!(
            "candidate XML topology mismatch: {candidate_matches}/{candidate_total}"
        ));
    }
    let structural = aidl::structural_diff(&source.model, &after);
    if !structural.is_empty() {
        return Err(format!(
            "candidate changes module structure: {}",
            structural.join("; ")
        ));
    }
    println!("audio_backend=aidl");
    println!("config_adapter=module-xml");
    println!("config_path={}", source.path.display());
    println!("policy={}", plan.policy);
    println!("runtime_module={}", runtime.name);
    println!("config_module={}", source.model.name);
    println!("source_topology={}/{}", source.matches, source.total);
    println!("candidate_topology={candidate_matches}/{candidate_total}");
    for target in &plan.targets {
        println!(
            "target={}|{}|{}|{}|{}",
            target.module, target.port, target.behavior, target.rate, target.pcm_type
        );
    }
    for (port, behavior, reason) in &plan.skipped {
        println!("skipped={port}|{behavior}|{reason}");
    }
    for change in aidl::profile_diff(&source.model, &after) {
        println!("profile_change={change}");
    }
    println!("structure_changes=0");
    println!("candidate_bytes={}", candidate.len());
    println!("apply_enabled=0");
    Ok(())
}

fn aidl_state_dir() -> PathBuf {
    state_root().join("aidl")
}

fn aidl_metadata_path() -> PathBuf {
    aidl_state_dir().join("active.conf")
}

fn aidl_original_path() -> PathBuf {
    aidl_state_dir().join("original.xml")
}

fn aidl_candidate_path() -> PathBuf {
    aidl_state_dir().join("current.xml")
}

fn read_aidl_target() -> Result<PathBuf, String> {
    let content = fs::read_to_string(aidl_metadata_path())
        .map_err(|error| format!("cannot read AIDL state metadata: {error}"))?;
    let target = content
        .lines()
        .find_map(|line| line.strip_prefix("target="))
        .filter(|value| value.starts_with('/') && !value.contains(".."))
        .ok_or_else(|| "AIDL state metadata has no safe target".to_string())?;
    Ok(PathBuf::from(target))
}

fn path_is_mounted(target: &Path) -> bool {
    let Ok(mountinfo) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let target = target.to_string_lossy();
    mountinfo.lines().any(|line| {
        line.split_whitespace()
            .nth(4)
            .is_some_and(|mountpoint| mountpoint == target)
    })
}

fn set_vendor_config_context(path: &Path) -> Result<(), String> {
    let status = Command::new("chcon")
        .args(["u:object_r:vendor_configs_file:s0"])
        .arg(path)
        .status()
        .map_err(|error| format!("cannot label {}: {error}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cannot label {} as vendor configuration",
            path.display()
        ))
    }
}

fn render_aidl_reload_script(target: &Path, candidate: Option<&Path>) -> String {
    let target = shell_quote(&target.to_string_lossy());
    let prepare = match candidate {
        Some(candidate) => format!(
            "if grep -Fq \" $target \" /proc/self/mountinfo; then umount \"$target\" || exit 31; fi\nmount --bind {} \"$target\" || exit 32\n",
            shell_quote(&candidate.to_string_lossy())
        ),
        None => "if grep -Fq \" $target \" /proc/self/mountinfo; then umount \"$target\" || exit 31; fi\n".to_string(),
    };
    let rollback = if candidate.is_some() {
        "    umount \"$target\" 2>/dev/null || true\n    setprop ctl.restart vendor.audio-hal-aidl\n    sleep 5\n"
    } else {
        ""
    };
    format!(
        "#!/system/bin/sh\nset -u\ntarget={target}\nold_hal=\"$(pidof audiohalservice.qti 2>/dev/null)\"\nold_audio=\"$(pidof audioserver 2>/dev/null)\"\n{prepare}setprop ctl.restart vendor.audio-hal-aidl\nok=0\nn=0\nwhile [ \"$n\" -lt 20 ]; do\n    new_hal=\"$(pidof audiohalservice.qti 2>/dev/null)\"\n    new_audio=\"$(pidof audioserver 2>/dev/null)\"\n    services=\"$(service list 2>/dev/null | grep -c android.hardware.audio.core)\"\n    if [ -n \"$new_hal\" ] && [ -n \"$new_audio\" ] && [ \"$new_hal\" != \"$old_hal\" ] && [ \"$new_audio\" != \"$old_audio\" ] && [ \"$services\" -ge 5 ]; then\n        ok=1\n        break\n    fi\n    sleep 1\n    n=$((n + 1))\ndone\nif [ \"$ok\" -ne 1 ]; then\n{rollback}    echo \"AIDL audio services did not recover\" >&2\n    exit 33\nfi\nprintf \"old_hal=%s\\nnew_hal=%s\\nold_audioserver=%s\\nnew_audioserver=%s\\naidl_services=%s\\n\" \"$old_hal\" \"$new_hal\" \"$old_audio\" \"$new_audio\" \"$services\"\n"
    )
}

fn validate_loaded_aidl_plan(
    snapshot: &aidl::AudioPolicySnapshot,
    plan: &aidl::PolicyPlan,
) -> Result<(), String> {
    for target in &plan.targets {
        let module = snapshot
            .modules
            .iter()
            .find(|module| module.name == target.module || module.name == "primary")
            .ok_or_else(|| format!("loaded module {} is missing", target.module))?;
        let port = module
            .output_mix_ports
            .iter()
            .find(|port| port.name == target.port)
            .ok_or_else(|| format!("loaded mix port {} is missing", target.port))?;
        let expected_format = match target.pcm_type.as_str() {
            "INT_16_BIT" => "AUDIO_FORMAT_PCM_16_BIT",
            "INT_24_BIT" => "AUDIO_FORMAT_PCM_24_BIT_PACKED",
            "INT_32_BIT" => "AUDIO_FORMAT_PCM_32_BIT",
            "FLOAT_32_BIT" => "AUDIO_FORMAT_PCM_FLOAT",
            value => return Err(format!("unsupported loaded PCM type: {value}")),
        };
        if !port.profiles.iter().any(|profile| {
            profile.rates.contains(&target.rate)
                && profile.format.as_deref() == Some(expected_format)
        }) {
            return Err(format!(
                "loaded mix port {} does not expose {} at {} Hz",
                target.port, expected_format, target.rate
            ));
        }
    }
    Ok(())
}

fn run_aidl_operation(settings: Settings, action: Action) -> Result<i32, String> {
    ensure_state_layout()?;
    fs::create_dir_all(aidl_state_dir())
        .map_err(|error| format!("cannot create AIDL state directory: {error}"))?;
    set_mode(&aidl_state_dir(), 0o700)?;
    let _lock = acquire_lock()?;

    if matches!(action, Action::Reset) {
        let target = match read_aidl_target() {
            Ok(target) => target,
            Err(_) => discover_aidl_primary_config(&audio_policy_snapshot()?)?.path,
        };
        let generated_path = state_root().join("generated/aidl-reset.sh");
        let generated = render_aidl_reload_script(&target, None);
        atomic_write(&generated_path, generated.as_bytes(), 0o700)?;
        let namespace = namespace_info();
        let (route, output) = execute_generated(&generated_path, &namespace)?;
        let code = output.status.code().unwrap_or(1);
        write_named_operation_log("aidl-reset", route, code, &generated_path, &output)?;
        if code == 0 {
            let _ = fs::remove_file(aidl_metadata_path());
            let _ = fs::remove_file(aidl_candidate_path());
            let _ = fs::remove_file(aidl_original_path());
            reset_policy_settings()?;
        }
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return Ok(code);
    }

    let snapshot = audio_policy_snapshot()?;
    let source = discover_aidl_primary_config(&snapshot)?;
    if path_is_mounted(&source.path) && !aidl_metadata_path().is_file() {
        return Err(format!(
            "{} is already mounted by another source; refusing to stack an AIDL writer",
            source.path.display()
        ));
    }
    let plan = aidl::plan_policy(
        &snapshot,
        &settings.policy,
        settings.sample_rate,
        &settings.bit_depth,
    )?;
    let candidate = aidl::apply_plan_to_module_xml(&source.xml, &source.model.name, &plan)?;
    let candidate_model = aidl::parse_module_xml(&candidate)?;
    let runtime = snapshot
        .modules
        .iter()
        .find(|module| module.name == "primary" || module.name == "default")
        .ok_or_else(|| "runtime primary/default module was not found".to_string())?;
    let (matched, total) = aidl::topology_match_score(runtime, &candidate_model);
    if total == 0
        || matched != total
        || !aidl::structural_diff(&source.model, &candidate_model).is_empty()
    {
        return Err(format!(
            "candidate failed final topology validation: {matched}/{total}"
        ));
    }
    atomic_write(&aidl_original_path(), source.xml.as_bytes(), 0o600)?;
    atomic_write(&aidl_candidate_path(), candidate.as_bytes(), 0o644)?;
    set_vendor_config_context(&aidl_candidate_path())?;
    atomic_write(
        &aidl_metadata_path(),
        format!("version=1\ntarget={}\n", source.path.display()).as_bytes(),
        0o600,
    )?;

    let generated_path = state_root().join("generated/aidl-apply.sh");
    let generated = render_aidl_reload_script(&source.path, Some(&aidl_candidate_path()));
    atomic_write(&generated_path, generated.as_bytes(), 0o700)?;
    let namespace = namespace_info();
    let (route, output) = execute_generated(&generated_path, &namespace)?;
    let mut code = output.status.code().unwrap_or(1);
    if code == 0 {
        match audio_policy_snapshot().and_then(|loaded| {
            validate_loaded_aidl_plan(&loaded, &plan)?;
            Ok(loaded)
        }) {
            Ok(_) => save_policy_settings(&settings)?,
            Err(error) => {
                code = 34;
                let _ = Command::new("umount").arg(&source.path).status();
                let _ = Command::new("setprop")
                    .args(["ctl.restart", "vendor.audio-hal-aidl"])
                    .status();
                let _ = fs::remove_file(aidl_metadata_path());
                let _ = fs::remove_file(aidl_candidate_path());
                eprintln!("AIDL post-restart validation failed and was rolled back: {error}");
            }
        }
    }
    if code != 0 {
        let _ = fs::remove_file(aidl_metadata_path());
        let _ = fs::remove_file(aidl_candidate_path());
    }
    write_named_operation_log("aidl-apply", route, code, &generated_path, &output)?;
    println!("controller_action=aidl-apply");
    println!("controller_route={route}");
    println!("aidl_target={}", source.path.display());
    println!("aidl_topology={matched}/{total}");
    println!(
        "aidl_mount_active={}",
        bool_number(path_is_mounted(&source.path))
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(code)
}

fn run_aidl_restore() -> Result<i32, String> {
    if !aidl_metadata_path().is_file() || !aidl_candidate_path().is_file() {
        println!("aidl_restore=not-configured");
        return Ok(0);
    }
    let target = read_aidl_target()?;
    if path_is_mounted(&target) {
        println!("aidl_restore=already-mounted");
        return Ok(0);
    }
    let current = fs::read(&target).map_err(|error| {
        format!(
            "cannot read current AIDL source {}: {error}",
            target.display()
        )
    })?;
    let original = fs::read(aidl_original_path())
        .map_err(|error| format!("cannot read saved AIDL source: {error}"))?;
    if current != original {
        return Err("AIDL source changed after configuration; refusing boot restore until it is revalidated".to_string());
    }
    set_vendor_config_context(&aidl_candidate_path())?;
    let generated_path = state_root().join("generated/aidl-restore.sh");
    let generated = render_aidl_reload_script(&target, Some(&aidl_candidate_path()));
    atomic_write(&generated_path, generated.as_bytes(), 0o700)?;
    let namespace = namespace_info();
    let (route, output) = execute_generated(&generated_path, &namespace)?;
    let code = output.status.code().unwrap_or(1);
    write_named_operation_log("aidl-restore", route, code, &generated_path, &output)?;
    println!(
        "aidl_restore={}",
        if code == 0 { "applied" } else { "failed" }
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(code)
}

fn print_audio_backend_status() {
    match audio_policy_snapshot() {
        Ok(snapshot) if snapshot.is_aidl() => {
            println!("audio_backend=aidl");
            println!("audio_config_source={}", snapshot.config_source);
            println!("aidl_module_count={}", snapshot.modules.len());
            println!(
                "aidl_usb_output_device_count={}",
                snapshot.usb_output_devices().len()
            );
            println!(
                "aidl_usb_reachable_mix_port_count={}",
                snapshot.usb_reachable_mix_ports().len()
            );
            let ports = snapshot.usb_reachable_mix_ports();
            let behaviors = ports
                .iter()
                .map(|(_, port)| port.behavior())
                .collect::<BTreeSet<_>>();
            println!(
                "aidl_usb_port_behaviors={}",
                behaviors.into_iter().collect::<Vec<_>>().join(",")
            );
            println!(
                "aidl_usb_port_summary={}",
                ports
                    .iter()
                    .map(|(module, port)| format!("{module}:{}:{}", port.name, port.behavior()))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            for (policy, _, _) in POLICIES.iter().filter(|(policy, _, _)| *policy != "legacy") {
                let rates = DOCUMENTED_RATES
                    .iter()
                    .map(|(rate, _)| *rate)
                    .filter(|rate| aidl::plan_policy(&snapshot, policy, *rate, "32").is_ok())
                    .map(|rate| rate.to_string())
                    .collect::<Vec<_>>();
                println!(
                    "aidl_supported_rates_{}={}",
                    policy.replace('-', "_"),
                    rates.join(",")
                );
            }
            match discover_aidl_primary_config(&snapshot) {
                Ok(source) => {
                    println!("aidl_config_adapter=module-xml");
                    println!("aidl_config_path={}", source.path.display());
                    println!("aidl_config_topology={}/{}", source.matches, source.total);
                    println!("aidl_preview_supported=1");
                    println!("aidl_apply_supported=1");
                    println!(
                        "aidl_mount_active={}",
                        bool_number(path_is_mounted(&source.path))
                    );
                }
                Err(error) => {
                    println!("aidl_config_adapter=read-only");
                    println!("aidl_preview_supported=0");
                    println!("aidl_apply_supported=0");
                    println!("aidl_config_error={}", error.replace('\n', " "));
                }
            }
        }
        Ok(snapshot) => {
            println!("audio_backend=traditional");
            println!("audio_config_source={}", snapshot.config_source);
        }
        Err(error) => {
            println!("audio_backend=unknown");
            println!("audio_backend_error={}", error.replace('\n', " "));
        }
    }
}

fn print_aidl_inspect() -> Result<(), String> {
    let snapshot = audio_policy_snapshot()?;
    print_aidl_snapshot(&snapshot)
}

fn print_aidl_snapshot(snapshot: &aidl::AudioPolicySnapshot) -> Result<(), String> {
    if !snapshot.is_aidl() {
        return Err(format!(
            "active audio policy is not AIDL: {}",
            snapshot.config_source
        ));
    }
    println!("config_source={}", snapshot.config_source);
    for module in &snapshot.modules {
        println!("module={}", module.name);
        for port in &module.output_mix_ports {
            let rates = port.unique_rates();
            let dynamic = port.profiles.iter().any(|profile| {
                profile.dynamic_rate || profile.dynamic_format || profile.dynamic_channels
            });
            println!(
                "mix_port={}|{}|{}|{}|{}",
                module.name,
                port.name,
                port.behavior(),
                if dynamic { "dynamic" } else { "static" },
                rates
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        for port in &module.device_ports {
            println!(
                "device_port={}|{}|{}",
                module.name,
                port.name,
                port.device_type.as_deref().unwrap_or("")
            );
        }
        for route in &module.routes {
            println!(
                "route={}|{}|{}",
                module.name,
                route.sink,
                route.sources.join(",")
            );
        }
    }
    for (module, port) in snapshot.usb_reachable_mix_ports() {
        println!(
            "usb_reachable_mix_port={module}|{}|{}",
            port.name,
            port.behavior()
        );
    }
    Ok(())
}

fn print_stored_settings(settings: &StoredSettings) {
    println!(
        "policy_configured={}",
        bool_number(settings.policy_configured)
    );
    println!("bluetooth_hal={}", settings.bluetooth_hal);
    println!(
        "bluetooth_hal_configured={}",
        bool_number(settings.bluetooth_hal_configured)
    );
    println!("resampler_preset={}", settings.resampler_preset);
    println!(
        "resampler_configured={}",
        bool_number(settings.resampler_configured)
    );
    println!("resampler_bypass={}", settings.resampler_bypass);
    println!("resampler_cheat={}", bool_number(settings.resampler_cheat));
    println!("resampler_stop_band={}", settings.resampler_stop_band);
    println!("resampler_half_length={}", settings.resampler_half_length);
    println!("resampler_percent={}", settings.resampler_percent);
    println!("usb_period={}", settings.usb_period);
    println!(
        "usb_period_configured={}",
        bool_number(settings.usb_period_configured)
    );
    println!("diagnostic={}", settings.diagnostic);
    println!("diagnostic_all={}", bool_number(settings.diagnostic_all));
    println!("io_scheduler={}", settings.io_scheduler);
    println!("io_tone={}", settings.io_tone);
    println!("wifi_no_restart={}", bool_number(settings.wifi_no_restart));
    println!("auto_reapply={}", bool_number(settings.auto_reapply));
    for feature in JITTER_FEATURES {
        println!(
            "jitter_{feature}={}",
            bool_number(
                settings
                    .jitter_values
                    .get(*feature)
                    .copied()
                    .unwrap_or(false)
            )
        );
        println!(
            "jitter_{feature}_configured={}",
            bool_number(
                settings
                    .jitter_configured
                    .get(*feature)
                    .copied()
                    .unwrap_or(false)
            )
        );
    }
}

fn upstream_script_version(module_dir: &Path) -> String {
    fs::read_to_string(module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh"))
        .ok()
        .and_then(|source| {
            source.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("# Version:")
                    .map(str::trim)
                    .filter(|version| !version.is_empty())
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn print_settings(settings: &Settings) {
    println!("policy={}", settings.policy);
    println!("sample_rate={}", settings.sample_rate);
    println!("bit_depth={}", settings.bit_depth);
    println!("drc={}", bool_number(settings.drc));
    println!("force_usbv2={}", bool_number(settings.force_usbv2));
    println!(
        "force_bluetooth_qti={}",
        bool_number(settings.force_bluetooth_qti)
    );
    println!(
        "bluetooth_a2dp_connected={}",
        bool_number(bluetooth_a2dp_connected())
    );
    println!("amzm={}", bool_number(settings.amzm));
    println!("test={}", bool_number(settings.test));
    println!(
        "test_template={}",
        settings.test_template.as_deref().unwrap_or("")
    );
}

fn print_schema(module_dir: &Path) {
    println!("schema_version=1");
    println!("custom_rate_min=44100");
    println!("custom_rate_max=768000");
    println!("policies_begin");
    for (value, flag, label) in POLICIES {
        println!("policy={value}|{flag}|{label}");
    }
    println!("policies_end");
    println!("rates_begin");
    for (value, label) in DOCUMENTED_RATES {
        println!("rate={value}|{label}");
    }
    println!("rates_end");
    println!("bit_depths_begin");
    for (value, label) in BIT_DEPTHS {
        println!("bit_depth={value}|{label}");
    }
    println!("bit_depths_end");
    println!("switch=drc|--drc");
    println!("switch=force_usbv2|--force-usbv2");
    println!("switch=force_bluetooth_qti|--force-bluetooth-qti");
    println!("switch=amzm|--amzm");
    println!("switch=test|--test");
    println!("action=reset|--reset");
    print_templates(module_dir);
}

fn print_templates(module_dir: &Path) {
    println!("templates_begin");
    for template in collect_templates(&module_dir.join(CORE_DIR).join("templates")) {
        println!("template={template}");
    }
    println!("templates_end");
}

fn collect_templates(root: &Path) -> Vec<String> {
    fn walk(root: &Path, current: &Path, output: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                walk(root, &path, output);
            } else if file_type.is_file() && path.extension() == Some(OsStr::new("xml")) {
                if let Ok(relative) = path.strip_prefix(root) {
                    output.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }

    let mut templates = Vec::new();
    walk(root, root, &mut templates);
    templates.sort();
    templates
}

fn namespace_info() -> NamespaceInfo {
    let audio_pid = audioserver_pid();
    NamespaceInfo {
        self_ns: read_namespace_link(Path::new("/proc/self/ns/mnt")),
        audio_ns: audio_pid
            .and_then(|pid| read_namespace_link(Path::new(&format!("/proc/{pid}/ns/mnt")))),
        audio_pid,
    }
}

fn read_namespace_link(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|value| value.to_string_lossy().into_owned())
}

fn audioserver_pid() -> Option<u32> {
    let from_pidof = Command::new("pidof")
        .arg("audioserver")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .and_then(|pid| pid.parse().ok())
        });
    if from_pidof.is_some() {
        return from_pidof;
    }
    Command::new("getprop")
        .arg("init.svc_debug_pid.audioserver")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
}

fn print_logs() -> Result<(), String> {
    let path = state_root().join("last.log");
    if !path.exists() {
        println!("No operation has been recorded yet.");
        return Ok(());
    }
    let mut file = File::open(&path).map_err(|error| format!("cannot open log: {error}"))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|error| format!("cannot read log: {error}"))?;
    print!("{content}");
    Ok(())
}

fn print_generated() -> Result<(), String> {
    let path = state_root().join("generated").join("apply.sh");
    if !path.exists() {
        return Err("no generated apply script exists".to_string());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read generated script: {error}"))?;
    print!("{content}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    #[test]
    fn normalizes_every_documented_alias_family() {
        assert_eq!(normalize_sample_rate("44.1k").unwrap(), 44_100);
        assert_eq!(normalize_sample_rate("353k").unwrap(), 352_800);
        assert_eq!(normalize_sample_rate("706k").unwrap(), 705_600);
        assert_eq!(normalize_sample_rate("123456").unwrap(), 123_456);
        assert!(normalize_sample_rate("44099").is_err());
        assert!(normalize_sample_rate("768001").is_err());
    }

    #[test]
    fn parses_complete_controller_configuration() {
        let args = [
            "--policy",
            "offload-direct",
            "--sample-rate",
            "96k",
            "--bit-depth",
            "float",
            "--drc",
            "--force-usbv2",
            "--force-bluetooth-qti",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let parsed = parse_settings_args(&args).unwrap();
        assert_eq!(parsed.policy, "offload-direct");
        assert_eq!(parsed.sample_rate, 96_000);
        assert_eq!(parsed.bit_depth, "float");
        assert!(parsed.drc && parsed.force_usbv2 && parsed.force_bluetooth_qti);
    }

    #[test]
    fn renders_all_upstream_flags_in_a_guarded_script() {
        let settings = Settings {
            policy: "offload-direct".to_string(),
            sample_rate: 96_000,
            bit_depth: "24".to_string(),
            drc: true,
            force_usbv2: true,
            force_bluetooth_qti: true,
            amzm: false,
            test: false,
            test_template: None,
        };
        let script = render_script(&settings, &module_fixture(), Action::Apply);
        assert!(script.contains("readlink /proc/self/ns/mnt"));
        assert!(script.contains("/core/USB_SampleRate_Changer.sh"));
        assert!(script.contains("--offload-direct"));
        assert!(script.contains("--drc"));
        assert!(script.contains("--force-usbv2"));
        assert!(script.contains("--force-bluetooth-qti"));
        assert!(script.contains("'96000' '24'"));
    }

    #[test]
    fn rejects_conflicting_template_modes() {
        let settings = Settings {
            amzm: true,
            test: true,
            test_template: Some("offload_template.xml".to_string()),
            ..Settings::default()
        };
        assert!(validate_settings(&settings, &module_fixture()).is_err());
    }

    #[test]
    fn quotes_single_quotes_for_shell() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn reset_has_no_configuration_arguments() {
        assert_eq!(
            upstream_args(&Settings::default(), Action::Reset),
            ["--reset"]
        );
    }

    #[test]
    fn detects_a2dp_only_inside_connected_device_section() {
        let connected = "volume bt_a2dp\n- STREAM_MUSIC:\n  Devices: bt_a2dp(80)\n- STREAM_ALARM:\nConnected devices:\n  [DeviceInfo: type:0x80 (bt_a2dp) name:Headphones]\nAPM Connected device (A2DP sink only):\n";
        let disconnected = "volume bt_a2dp\n- STREAM_MUSIC:\n  Devices: speaker(2)\n- STREAM_ALARM:\nConnected devices:\n  [DeviceInfo: type:0x10 (bt_sco) name:Headphones]\nAPM Connected device (A2DP sink only):\n";
        let connected_but_unrouted = "- STREAM_MUSIC:\n  Devices: speaker(2)\n- STREAM_ALARM:\nConnected devices:\n  [DeviceInfo: type:0x80 (bt_a2dp) name:Headphones]\nAPM Connected device (A2DP sink only):\n";
        assert!(bluetooth_a2dp_connected_in_dump(connected));
        assert!(!bluetooth_a2dp_connected_in_dump(disconnected));
        assert!(!bluetooth_a2dp_connected_in_dump(connected_but_unrouted));
    }

    #[test]
    fn extra_commands_are_strictly_whitelisted() {
        let bluetooth = ["bluetooth-hal", "offload"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let parsed = parse_extra_command(&bluetooth).unwrap();
        assert_eq!(parsed.script, "change-bluetooth-hal.sh");
        assert_eq!(parsed.args, ["offload"]);

        let injection = ["bluetooth-hal", "offload;id"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(parse_extra_command(&injection).is_err());
    }

    #[test]
    fn validates_usb_period_and_jitter_parameters() {
        let valid = ["usb-period", "2250"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(parse_extra_command(&valid).is_ok());
        let invalid = ["usb-period", "2251"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(parse_extra_command(&invalid).is_err());

        let io = ["jitter", "enable", "io", "*", "boost"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            parse_extra_command(&io).unwrap().args,
            ["--io", "*", "boost", "--status"]
        );

        let custom = ["resampler", "custom", "96", "cheat", "194", "520", "98"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            parse_extra_command(&custom).unwrap().args,
            ["--bypass-hires", "--cheat", "194", "520", "98"]
        );

        let wifi = ["jitter", "enable", "wifi", "no-restart"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            parse_extra_command(&wifi).unwrap().args,
            ["--wifi-no-restart", "--status"]
        );
    }

    #[test]
    fn stored_settings_round_trip_all_persisted_values() {
        let mut settings = StoredSettings::default();
        settings.policy.policy = "usb".to_string();
        settings.policy_configured = true;
        settings.bluetooth_hal = "aosp".to_string();
        settings.bluetooth_hal_configured = true;
        settings.resampler_preset = "custom".to_string();
        settings.resampler_configured = true;
        settings.resampler_bypass = "96".to_string();
        settings.resampler_stop_band = 194;
        settings.resampler_half_length = 520;
        settings.resampler_percent = 98;
        settings.usb_period = 1_000;
        settings.usb_period_configured = true;
        settings.jitter_values.insert("io".to_string(), true);
        settings.jitter_configured.insert("io".to_string(), true);
        settings.io_scheduler = "bfq".to_string();
        settings.io_tone = "boost".to_string();
        settings.auto_reapply = true;

        assert_eq!(
            parse_stored_settings(&render_stored_settings(&settings)),
            settings
        );
    }

    #[test]
    fn legacy_settings_are_treated_as_an_applied_policy() {
        let settings =
            parse_stored_settings("version=2\npolicy=usb\nsample_rate=96000\nbit_depth=24\n");
        assert!(settings.policy_configured);
        assert_eq!(settings.policy.policy, "usb");
        assert_eq!(settings.policy.sample_rate, 96_000);
        assert_eq!(settings.policy.bit_depth, "24");
    }

    #[test]
    fn extra_updates_every_tool_and_tuning_selection() {
        let mut settings = StoredSettings::default();
        let policy = settings.policy.clone();
        let bluetooth = ["bluetooth-hal", "aosp"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &bluetooth));
        assert!(settings.bluetooth_hal_configured);
        assert_eq!(settings.bluetooth_hal, "aosp");

        let custom = ["resampler", "custom", "96", "cheat", "194", "520", "98"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        assert!(update_stored_settings_for_extra(&mut settings, &custom));
        assert!(settings.resampler_configured);
        assert_eq!(settings.resampler_preset, "custom");
        assert_eq!(settings.resampler_bypass, "96");
        assert_eq!(settings.resampler_stop_band, 194);
        assert_eq!(settings.resampler_half_length, 520);
        assert_eq!(settings.resampler_percent, 98);
        assert_eq!(settings.policy, policy);

        let preset = ["resampler", "ultra-hifi"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &preset));
        assert_eq!(settings.resampler_preset, "ultra-hifi");
        assert_eq!(settings.resampler_bypass, "none");
        assert!(settings.resampler_cheat);
        assert_eq!(settings.resampler_stop_band, 194);
        assert_eq!(settings.resampler_half_length, 520);
        assert_eq!(settings.resampler_percent, 98);

        let usb = ["usb-period", "1000"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &usb));
        assert!(settings.usb_period_configured);
        assert_eq!(settings.usb_period, 1_000);

        let diagnostic = ["diagnose", "alsa", "all"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &diagnostic));
        assert_eq!(settings.diagnostic, "alsa");
        assert!(settings.diagnostic_all);

        let io = ["jitter", "enable", "io", "bfq", "boost"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &io));
        assert_eq!(settings.jitter_values.get("io"), Some(&true));
        assert_eq!(settings.jitter_configured.get("io"), Some(&true));
        assert_eq!(settings.io_scheduler, "bfq");
        assert_eq!(settings.io_tone, "boost");

        let wifi = ["jitter", "enable", "wifi", "no-restart"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &wifi));
        assert_eq!(settings.jitter_values.get("wifi"), Some(&true));
        assert!(settings.wifi_no_restart);

        let disable_battery = ["jitter", "disable", "battery"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(
            &mut settings,
            &disable_battery
        ));
        assert_eq!(settings.jitter_values.get("battery"), Some(&false));
        assert_eq!(settings.jitter_configured.get("battery"), Some(&true));

        let reset_resampler = ["resampler", "reset"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(
            &mut settings,
            &reset_resampler
        ));
        assert!(!settings.resampler_configured);

        let reset_usb = ["usb-period", "reset"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(&mut settings, &reset_usb));
        assert!(!settings.usb_period_configured);

        let reset_jitter = ["jitter", "disable", "all"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(update_stored_settings_for_extra(
            &mut settings,
            &reset_jitter
        ));
        for feature in JITTER_BASE_FEATURES {
            assert_eq!(settings.jitter_values.get(*feature), Some(&false));
            assert_eq!(settings.jitter_configured.get(*feature), Some(&false));
        }
    }

    #[test]
    fn reapply_plan_replays_only_applied_values_in_dependency_order() {
        let mut settings = StoredSettings::default();
        settings.bluetooth_hal = "aosp".to_string();
        settings.bluetooth_hal_configured = true;
        settings.policy.policy = "usb".to_string();
        settings.policy_configured = true;
        settings.resampler_preset = "custom".to_string();
        settings.resampler_configured = true;
        settings.resampler_bypass = "96".to_string();
        settings.resampler_stop_band = 194;
        settings.resampler_half_length = 520;
        settings.resampler_percent = 98;
        settings.usb_period = 1_000;
        settings.usb_period_configured = true;
        settings.jitter_values.insert("thermal".to_string(), false);
        settings
            .jitter_configured
            .insert("thermal".to_string(), true);
        settings.jitter_values.insert("io".to_string(), true);
        settings.jitter_configured.insert("io".to_string(), true);
        settings.io_scheduler = "bfq".to_string();
        settings.io_tone = "boost".to_string();
        settings.diagnostic = "alsa".to_string();

        assert!(build_reapply_plan(&settings).is_empty());
        settings.auto_reapply = true;
        assert_eq!(
            build_reapply_plan(&settings),
            vec![
                ReapplyAction::Extra(vec!["bluetooth-hal".into(), "aosp".into()]),
                ReapplyAction::Policy(settings.policy.clone()),
                ReapplyAction::Extra(vec![
                    "resampler".into(),
                    "custom".into(),
                    "96".into(),
                    "cheat".into(),
                    "194".into(),
                    "520".into(),
                    "98".into(),
                ]),
                ReapplyAction::Extra(vec!["usb-period".into(), "1000".into()]),
                ReapplyAction::Extra(vec!["jitter".into(), "disable".into(), "thermal".into(),]),
                ReapplyAction::Extra(vec![
                    "jitter".into(),
                    "enable".into(),
                    "io".into(),
                    "bfq".into(),
                    "boost".into(),
                ]),
            ]
        );
    }
}
