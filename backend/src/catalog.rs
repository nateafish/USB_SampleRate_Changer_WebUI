pub(crate) const JITTER_FEATURES: &[&str] = &[
    "selinux", "thermal", "doze", "governor", "camera", "logd", "io", "vm", "wifi", "battery",
    "effect",
];

pub(crate) const JITTER_BASE_FEATURES: &[&str] = &[
    "selinux", "thermal", "doze", "governor", "camera", "logd", "io", "vm", "wifi",
];

pub(crate) const IO_SCHEDULERS: &[&str] = &[
    "*",
    "none",
    "noop",
    "deadline",
    "mq-deadline",
    "cfq",
    "bfq",
    "kyber",
];

pub(crate) const IO_TONES: &[&str] = &["light", "m-light", "medium", "boost", "exp"];

pub(crate) const BLUETOOTH_HAL_OPTIONS: &[&str] = &["offload", "aosp", "legacy", "sysbta"];

pub(crate) const DIAGNOSTIC_TYPES: &[&str] = &["audio", "bluetooth", "config", "alsa"];

pub(crate) const RESAMPLER_BYPASSES: &[(&str, u32)] =
    &[("none", 44_100), ("48", 48_000), ("96", 96_000)];

pub(crate) const RESAMPLER_MODES: &[&str] = &["cutoff", "cheat"];

pub(crate) const RESAMPLER_PRESET_GROUPS: &[(&str, &[&str])] = &[
    (
        "standard",
        &[
            "159-480-92",
            "165-360-104",
            "179-408-99",
            "194-520-100",
            "ultra-hifi",
            "custom",
        ],
    ),
    (
        "nonlinear",
        &[
            "cheap-44",
            "cheap-44-low",
            "cheap-48",
            "cheap-48-low",
            "cheap-96",
        ],
    ),
    (
        "simulated",
        &["mock-dac-a", "mock-dac-b", "mock-dac-c", "mock-mastering"],
    ),
];

pub(crate) const POLICIES: &[(&str, &str, &str)] = &[
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

pub(crate) const DOCUMENTED_RATES: &[(u32, &str)] = &[
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

pub(crate) const BIT_DEPTHS: &[(&str, &str)] = &[
    ("16", "16-bit PCM"),
    ("24", "24-bit packed PCM"),
    ("32", "32-bit PCM"),
    ("float", "32-bit float PCM"),
];

pub(crate) const RESAMPLER_PRESETS: &[(&str, &[&str])] = &[
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

pub(crate) const RESAMPLER_DEFAULT_PRESET: &str = "179-408-99";

// The script default option maps to the same AudioFlinger preset as Android
// 12+ (default), so exposing a separate System default entry would be
// duplicate. Keep accepting the historical CLI value without advertising it
// as a selectable schema preset.
pub(crate) fn canonical_resampler_preset(value: &str) -> &str {
    if value == "default" {
        RESAMPLER_DEFAULT_PRESET
    } else {
        value
    }
}

pub(crate) fn resampler_preset_args(value: &str) -> Option<&'static [&'static str]> {
    let value = canonical_resampler_preset(value);
    RESAMPLER_PRESETS
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, args)| *args)
}
