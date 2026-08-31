use std::collections::BTreeMap;

use crate::catalog::{resampler_preset_args, JITTER_FEATURES};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) policy: String,
    pub(crate) sample_rate: u32,
    pub(crate) bit_depth: String,
    pub(crate) drc: bool,
    pub(crate) force_usbv2: bool,
    pub(crate) force_bluetooth_qti: bool,
    pub(crate) amzm: bool,
    pub(crate) test: bool,
    pub(crate) test_template: Option<String>,
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
pub(crate) struct StoredSettings {
    pub(crate) policy: Settings,
    pub(crate) policy_configured: bool,
    pub(crate) bluetooth_hal: String,
    pub(crate) bluetooth_hal_configured: bool,
    pub(crate) resampler_preset: String,
    pub(crate) resampler_configured: bool,
    pub(crate) resampler_bypass: String,
    pub(crate) resampler_cheat: bool,
    pub(crate) resampler_stop_band: u32,
    pub(crate) resampler_half_length: u32,
    pub(crate) resampler_percent: u32,
    pub(crate) usb_period: u32,
    pub(crate) usb_period_configured: bool,
    pub(crate) diagnostic: String,
    pub(crate) diagnostic_all: bool,
    pub(crate) jitter_values: BTreeMap<String, bool>,
    pub(crate) jitter_configured: BTreeMap<String, bool>,
    pub(crate) io_scheduler: String,
    pub(crate) io_tone: String,
    pub(crate) wifi_no_restart: bool,
    pub(crate) auto_reapply: bool,
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
pub(crate) enum ExtraAction {
    BluetoothHal {
        action: String,
    },
    ResamplerStatus,
    ResamplerReset,
    ResamplerPreset {
        preset: String,
    },
    ResamplerCustom {
        bypass: String,
        cheat: bool,
        stop_band: u32,
        half_length: u32,
        percent: u32,
    },
    UsbPeriodStatus,
    UsbPeriodReset,
    UsbPeriodSet {
        period: u32,
    },
    JitterStatus,
    JitterSet {
        enabled: bool,
        feature: String,
        scheduler: Option<String>,
        tone: Option<String>,
        wifi_no_restart: bool,
    },
    Diagnose {
        kind: String,
        all: bool,
    },
}

impl ExtraAction {
    pub(crate) fn tool(&self) -> String {
        match self {
            Self::BluetoothHal { .. } => "bluetooth-hal".to_string(),
            Self::ResamplerStatus
            | Self::ResamplerReset
            | Self::ResamplerPreset { .. }
            | Self::ResamplerCustom { .. } => "resampler".to_string(),
            Self::UsbPeriodStatus | Self::UsbPeriodReset | Self::UsbPeriodSet { .. } => {
                "usb-period".to_string()
            }
            Self::JitterStatus | Self::JitterSet { .. } => "jitter".to_string(),
            Self::Diagnose { kind, .. } => format!("diagnose-{kind}"),
        }
    }

    pub(crate) fn script(&self) -> &'static str {
        match self {
            Self::BluetoothHal { .. } => "change-bluetooth-hal.sh",
            Self::ResamplerStatus
            | Self::ResamplerReset
            | Self::ResamplerPreset { .. }
            | Self::ResamplerCustom { .. } => "change-resampling-quality.sh",
            Self::UsbPeriodStatus | Self::UsbPeriodReset | Self::UsbPeriodSet { .. } => {
                "change-usb-period.sh"
            }
            Self::JitterStatus | Self::JitterSet { .. } => "jitter-reducer.sh",
            Self::Diagnose { kind, .. } => match kind.as_str() {
                "audio" => "dumpsys-filtered.sh",
                "bluetooth" => "dumpsys-bluetooth-filtered.sh",
                "config" => "getConfig.sh",
                "alsa" => "alsa-hw-params.sh",
                _ => unreachable!("diagnostic kind is validated by the CLI parser"),
            },
        }
    }

    pub(crate) fn script_args(&self) -> Vec<String> {
        match self {
            Self::BluetoothHal { action } => vec![if action == "status" {
                "--status".to_string()
            } else {
                action.clone()
            }],
            Self::ResamplerStatus => vec!["--status".to_string()],
            Self::ResamplerReset => vec!["--reset".to_string()],
            Self::ResamplerPreset { preset } => resampler_preset_args(preset)
                .map(|args| args.iter().map(|value| (*value).to_string()).collect())
                .unwrap_or_default(),
            Self::ResamplerCustom {
                bypass,
                cheat,
                stop_band,
                half_length,
                percent,
            } => {
                let mut args = Vec::new();
                match bypass.as_str() {
                    "48" => args.push("--bypass".to_string()),
                    "96" => args.push("--bypass-hires".to_string()),
                    _ => {}
                }
                if *cheat {
                    args.push("--cheat".to_string());
                }
                args.extend([
                    stop_band.to_string(),
                    half_length.to_string(),
                    percent.to_string(),
                ]);
                args
            }
            Self::UsbPeriodStatus => vec!["--status".to_string()],
            Self::UsbPeriodReset => vec!["--reset".to_string()],
            Self::UsbPeriodSet { period } => vec![period.to_string()],
            Self::JitterStatus => vec!["--status".to_string()],
            Self::JitterSet {
                enabled,
                feature,
                scheduler,
                tone,
                wifi_no_restart,
            } => {
                let flag = if feature == "wifi" && *enabled && *wifi_no_restart {
                    "--wifi-no-restart".to_string()
                } else if feature == "all" {
                    if *enabled { "--all" } else { "++all" }.to_string()
                } else {
                    format!("{}{}", if *enabled { "--" } else { "++" }, feature)
                };
                let mut args = vec![flag];
                if feature == "io" && *enabled {
                    args.push(scheduler.clone().unwrap_or_else(|| "*".to_string()));
                    args.push(tone.clone().unwrap_or_else(|| "medium".to_string()));
                }
                args.push("--status".to_string());
                args
            }
            Self::Diagnose { all, .. } => {
                if *all {
                    vec!["--all".to_string()]
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub(crate) fn requires_a2dp_post_check(&self) -> bool {
        // Jitter's effect mode restarts audioserver, but it does not change
        // routing configuration and therefore intentionally skips A2DP
        // verification. Every other mutating extra follows restart behavior.
        !matches!(self, Self::JitterStatus | Self::JitterSet { .. })
            && self.requires_audio_restart()
    }

    pub(crate) fn persists_state(&self) -> bool {
        !matches!(
            self,
            Self::BluetoothHal { action } if action == "status"
        ) && !matches!(
            self,
            Self::ResamplerStatus | Self::UsbPeriodStatus | Self::JitterStatus
        )
    }

    pub(crate) fn is_query(&self) -> bool {
        matches!(
            self,
            Self::BluetoothHal { action } if action == "status"
        ) || matches!(
            self,
            Self::ResamplerStatus
                | Self::UsbPeriodStatus
                | Self::JitterStatus
                | Self::Diagnose { .. }
        )
    }

    pub(crate) fn requires_audio_restart(&self) -> bool {
        match self {
            Self::BluetoothHal { action } => action != "status",
            Self::ResamplerReset
            | Self::ResamplerPreset { .. }
            | Self::ResamplerCustom { .. }
            | Self::UsbPeriodReset
            | Self::UsbPeriodSet { .. } => true,
            Self::JitterSet { feature, .. } => feature == "effect",
            Self::ResamplerStatus
            | Self::UsbPeriodStatus
            | Self::JitterStatus
            | Self::Diagnose { .. } => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReapplyAction {
    Policy(Settings),
    Extra(ExtraAction),
}

impl ReapplyAction {
    pub(crate) fn requires_audio_restart(&self) -> bool {
        match self {
            Self::Policy(_) => true,
            Self::Extra(action) => action.requires_audio_restart(),
        }
    }

    pub(crate) fn requires_a2dp_post_check(&self) -> bool {
        match self {
            Self::Policy(_) => true,
            Self::Extra(action) => action.requires_a2dp_post_check(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NamespaceInfo {
    pub(crate) self_ns: Option<String>,
    pub(crate) audio_ns: Option<String>,
    pub(crate) audio_pid: Option<u32>,
}

impl NamespaceInfo {
    pub(crate) fn matches(&self) -> Option<bool> {
        match (&self.self_ns, &self.audio_ns) {
            (Some(left), Some(right)) => Some(left == right),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Apply,
    Reset,
}

impl Action {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Reset => "reset",
        }
    }
}
