use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::catalog::{
    canonical_resampler_preset, BIT_DEPTHS, IO_SCHEDULERS, IO_TONES, JITTER_BASE_FEATURES,
    JITTER_FEATURES, POLICIES, RESAMPLER_PRESETS,
};
use crate::cli::normalize_sample_rate;
use crate::domain::{ExtraAction, Settings, StoredSettings};
use crate::paths::{atomic_write, state_root};

const SETTINGS_VERSION: &str = "3";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StateHealth {
    Healthy,
    Degraded(String),
}

impl StateHealth {
    pub(crate) fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded(_))
    }

    pub(crate) fn reason(&self) -> Option<&str> {
        match self {
            Self::Healthy => None,
            Self::Degraded(reason) => Some(reason),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StateSnapshot {
    pub(crate) settings: StoredSettings,
    pub(crate) health: StateHealth,
    pub(crate) migrated_from: Option<u32>,
    pub(crate) recovery_reason: Option<String>,
}

type StateWriter = fn(&Path, &[u8], u32) -> Result<(), String>;

pub(crate) struct StateStore {
    path: PathBuf,
    writer: StateWriter,
}

impl StateStore {
    pub(crate) fn new() -> Self {
        Self::at(state_root().join("settings.conf"))
    }

    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path,
            writer: atomic_write,
        }
    }

    #[cfg(test)]
    pub(crate) fn at_with_writer(path: PathBuf, writer: StateWriter) -> Self {
        Self { path, writer }
    }

    pub(crate) fn load(&self) -> StateSnapshot {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return StateSnapshot {
                    settings: StoredSettings::default(),
                    health: StateHealth::Healthy,
                    migrated_from: None,
                    recovery_reason: None,
                };
            }
            Err(error) => {
                return StateSnapshot {
                    settings: StoredSettings::default(),
                    health: StateHealth::Degraded(format!("cannot read state: {error}")),
                    migrated_from: None,
                    recovery_reason: None,
                };
            }
        };

        match parse_stored_settings_checked(&content) {
            Ok(LoadOutcome::Current(settings)) => StateSnapshot {
                settings,
                health: StateHealth::Healthy,
                migrated_from: None,
                recovery_reason: None,
            },
            Ok(LoadOutcome::Rewrite {
                settings,
                migrated_from,
                recovery_reason,
            }) => match self.save(&settings) {
                Ok(()) => StateSnapshot {
                    settings,
                    health: StateHealth::Healthy,
                    migrated_from,
                    recovery_reason,
                },
                Err(error) => StateSnapshot {
                    settings: StoredSettings::default(),
                    health: StateHealth::Degraded(format!("state rewrite failed: {error}")),
                    migrated_from: None,
                    recovery_reason: None,
                },
            },
            Err(reason) => StateSnapshot {
                settings: StoredSettings::default(),
                health: StateHealth::Degraded(reason),
                migrated_from: None,
                recovery_reason: None,
            },
        }
    }

    pub(crate) fn save(&self, settings: &StoredSettings) -> Result<(), String> {
        (self.writer)(
            &self.path,
            render_stored_settings(settings).as_bytes(),
            0o600,
        )
    }

    pub(crate) fn update(&self, delta: StateDelta) -> Result<StateSnapshot, String> {
        let mut snapshot = self.load();
        if let Some(reason) = snapshot.health.reason().map(str::to_owned) {
            // Never replace an unreadable state with defaults implicitly. A
            // deliberate policy reset is the one operation that can recover
            // the file in place; all other deltas surface the degraded state.
            if !matches!(&delta, StateDelta::PolicyReset) {
                return Err(format!("state is degraded: {reason}"));
            }
        }
        if delta.apply(&mut snapshot.settings) {
            self.save(&snapshot.settings)?;
            snapshot.health = StateHealth::Healthy;
            snapshot.migrated_from = None;
            snapshot.recovery_reason = None;
        }
        Ok(snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StateDelta {
    PolicyApply(Settings),
    PolicyReset,
    Extra(ExtraAction),
    SetAutoReapply(bool),
}

impl StateDelta {
    fn apply(&self, settings: &mut StoredSettings) -> bool {
        match self {
            Self::PolicyApply(policy) => {
                settings.policy = policy.clone();
                settings.policy_configured = true;
                true
            }
            Self::PolicyReset => {
                settings.policy = Settings::default();
                settings.policy_configured = false;
                true
            }
            Self::Extra(action) => update_stored_settings_for_extra(settings, action),
            Self::SetAutoReapply(enabled) => {
                settings.auto_reapply = *enabled;
                true
            }
        }
    }
}

pub(crate) fn bool_number(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}

pub(crate) fn render_stored_settings(settings: &StoredSettings) -> String {
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
        canonical_resampler_preset(&settings.resampler_preset),
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

const FIXED_STATE_KEYS: &[&str] = &[
    "version",
    "policy_configured",
    "policy",
    "sample_rate",
    "bit_depth",
    "drc",
    "force_usbv2",
    "force_bluetooth_qti",
    "amzm",
    "test",
    "test_template",
    "bluetooth_hal_configured",
    "bluetooth_hal",
    "resampler_configured",
    "resampler_preset",
    "resampler_bypass",
    "resampler_cheat",
    "resampler_stop_band",
    "resampler_half_length",
    "resampler_percent",
    "usb_period_configured",
    "usb_period",
    "diagnostic",
    "diagnostic_all",
    "io_scheduler",
    "io_tone",
    "wifi_no_restart",
    "auto_reapply",
];

const RESAMPLER_CUSTOM_KEYS: &[&str] = &[
    "resampler_bypass",
    "resampler_cheat",
    "resampler_stop_band",
    "resampler_half_length",
    "resampler_percent",
];

#[derive(Debug)]
enum LoadOutcome {
    Current(StoredSettings),
    Rewrite {
        settings: StoredSettings,
        migrated_from: Option<u32>,
        recovery_reason: Option<String>,
    },
}

#[derive(Clone, Copy)]
struct StateEntry<'a> {
    line_number: usize,
    key: &'a str,
    value: &'a str,
}

#[cfg(test)]
pub(crate) fn parse_stored_settings(content: &str) -> StoredSettings {
    match parse_stored_settings_checked(content) {
        Ok(LoadOutcome::Current(settings) | LoadOutcome::Rewrite { settings, .. }) => settings,
        Err(_) => StoredSettings::default(),
    }
}

fn parse_stored_settings_checked(content: &str) -> Result<LoadOutcome, String> {
    match detect_state_version(content)? {
        None => {
            let (settings, reason) = salvage_unversioned(content);
            Ok(LoadOutcome::Rewrite {
                settings,
                migrated_from: None,
                recovery_reason: Some(reason),
            })
        }
        Some(3) => parse_v3_strict(content).map(LoadOutcome::Current),
        Some(version @ (1 | 2)) => {
            let settings = parse_versioned_legacy(content)?;
            Ok(LoadOutcome::Rewrite {
                settings,
                migrated_from: Some(version),
                recovery_reason: None,
            })
        }
        Some(version) => Err(format!("unsupported state version: {version}")),
    }
}

fn detect_state_version(content: &str) -> Result<Option<u32>, String> {
    let mut version = None;
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key != "version" {
            continue;
        }
        if version.is_some() {
            return Err("duplicate state key: version".to_string());
        }
        version = Some(
            value
                .parse::<u32>()
                .map_err(|_| format!("invalid state version: {value}"))?,
        );
    }
    Ok(version)
}

fn parse_entries_strict(content: &str) -> Result<Vec<StateEntry<'_>>, String> {
    content
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line_number = index + 1;
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("malformed state line {line_number}"))?;
            if key.is_empty() {
                return Err(format!("empty state key on line {line_number}"));
            }
            Ok(StateEntry {
                line_number,
                key,
                value,
            })
        })
        .collect()
}

fn parse_v3_strict(content: &str) -> Result<StoredSettings, String> {
    let entries = parse_entries_strict(content)?;
    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !is_known_state_key(entry.key) {
            return Err(format!(
                "unknown state key on line {}: {}",
                entry.line_number, entry.key
            ));
        }
        if !seen.insert(entry.key) {
            return Err(format!("duplicate state key: {}", entry.key));
        }
    }
    for key in required_state_keys() {
        if !seen.contains(key.as_str()) {
            return Err(format!("missing state key: {key}"));
        }
    }

    let mut settings = StoredSettings::default();
    for entry in entries {
        apply_state_value(&mut settings, entry.key, entry.value)?;
    }
    validate_resampler_mode(&settings)?;
    Ok(settings)
}

fn parse_versioned_legacy(content: &str) -> Result<StoredSettings, String> {
    let entries = parse_entries_strict(content)?;
    let observed: BTreeSet<&str> = entries.iter().map(|entry| entry.key).collect();
    let mut settings = StoredSettings {
        policy_configured: !content.trim().is_empty() && !observed.contains("policy_configured"),
        ..StoredSettings::default()
    };
    for entry in entries {
        if is_known_state_key(entry.key) {
            apply_state_value(&mut settings, entry.key, entry.value)?;
        }
    }
    normalize_legacy_resampler(&mut settings, &observed);
    Ok(settings)
}

fn salvage_unversioned(content: &str) -> (StoredSettings, String) {
    let mut settings = StoredSettings::default();
    let mut accepted = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut ignored = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let Some((key, value)) = line.split_once('=') else {
            ignored.push(format!("malformed line {line_number}"));
            continue;
        };
        if key.is_empty() {
            ignored.push(format!("empty key on line {line_number}"));
            continue;
        }
        if !is_known_state_key(key) || key == "version" {
            ignored.push(format!("unknown key {key}"));
            continue;
        }
        if !seen.insert(key) {
            ignored.push(format!("duplicate key {key}"));
        }
        match apply_state_value(&mut settings, key, value) {
            Ok(()) => {
                accepted.insert(key);
            }
            Err(error) => ignored.push(error),
        }
    }

    if settings.resampler_preset == "custom" {
        let complete = RESAMPLER_CUSTOM_KEYS
            .iter()
            .all(|key| accepted.contains(key));
        if !complete || validate_resampler_mode(&settings).is_err() {
            reset_resampler_to_canonical_default(&mut settings);
            ignored.push("incomplete or invalid custom resampler group".to_string());
        }
    } else if validate_resampler_mode(&settings).is_err() {
        settings.resampler_percent = StoredSettings::default().resampler_percent;
        ignored.push("resampler percent does not match cutoff mode".to_string());
    }

    let reason = if content.is_empty() {
        "empty unversioned state rebuilt from defaults".to_string()
    } else if ignored.is_empty() {
        "unversioned legacy state rebuilt as complete v3".to_string()
    } else {
        let shown = ignored
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let suffix = if ignored.len() > 4 {
            format!("; and {} more", ignored.len() - 4)
        } else {
            String::new()
        };
        format!(
            "unversioned legacy state salvaged; ignored {} entry(s): {shown}{suffix}",
            ignored.len()
        )
    };
    (settings, reason)
}

fn apply_state_value(settings: &mut StoredSettings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "version" => {}
        "policy_configured" => settings.policy_configured = parse_bool(key, value)?,
        "policy" if POLICIES.iter().any(|(name, _, _)| *name == value) => {
            settings.policy.policy = value.to_string();
        }
        "policy" => return Err(format!("unsupported policy in state: {value}")),
        "sample_rate" => {
            value
                .parse::<u32>()
                .map_err(|_| format!("invalid state sample rate: {value}"))?;
            settings.policy.sample_rate = normalize_sample_rate(value)
                .map_err(|error| format!("invalid state sample rate: {error}"))?;
        }
        "bit_depth" if BIT_DEPTHS.iter().any(|(depth, _)| *depth == value) => {
            settings.policy.bit_depth = value.to_string();
        }
        "bit_depth" => return Err(format!("unsupported bit depth in state: {value}")),
        "drc" => settings.policy.drc = parse_bool(key, value)?,
        "force_usbv2" => settings.policy.force_usbv2 = parse_bool(key, value)?,
        "force_bluetooth_qti" => {
            settings.policy.force_bluetooth_qti = parse_bool(key, value)?;
        }
        "amzm" => settings.policy.amzm = parse_bool(key, value)?,
        "test" => settings.policy.test = parse_bool(key, value)?,
        "test_template" => {
            if value.contains(['\n', '\r']) {
                return Err("invalid test template in state".to_string());
            }
            settings.policy.test_template = (!value.is_empty()).then(|| value.to_string());
        }
        "bluetooth_hal_configured" => {
            settings.bluetooth_hal_configured = parse_bool(key, value)?;
        }
        "bluetooth_hal" if matches!(value, "aosp" | "legacy" | "offload" | "sysbta") => {
            settings.bluetooth_hal = value.to_string();
        }
        "bluetooth_hal" => {
            return Err(format!("unsupported Bluetooth HAL in state: {value}"));
        }
        "resampler_configured" => settings.resampler_configured = parse_bool(key, value)?,
        "resampler_preset" => {
            let canonical = canonical_resampler_preset(value);
            if canonical != "custom"
                && !RESAMPLER_PRESETS.iter().any(|(name, _)| *name == canonical)
            {
                return Err(format!("unsupported resampler preset in state: {value}"));
            }
            settings.resampler_preset = canonical.to_string();
        }
        "resampler_bypass" if matches!(value, "none" | "48" | "96") => {
            settings.resampler_bypass = value.to_string();
        }
        "resampler_bypass" => {
            return Err(format!("unsupported resampler bypass in state: {value}"));
        }
        "resampler_cheat" => settings.resampler_cheat = parse_bool(key, value)?,
        "resampler_stop_band" => {
            settings.resampler_stop_band = validate_u32(value, 20..=242, "resampler stop band")?;
        }
        "resampler_half_length" => {
            let parsed = validate_u32(value, 8..=640, "resampler half length")?;
            if parsed % 8 != 0 {
                return Err(format!(
                    "resampler half length is not a multiple of 8: {parsed}"
                ));
            }
            settings.resampler_half_length = parsed;
        }
        "resampler_percent" => {
            settings.resampler_percent = validate_u32(value, 1..=200, "resampler percent")?;
        }
        "usb_period_configured" => settings.usb_period_configured = parse_bool(key, value)?,
        "usb_period" => {
            let parsed = validate_u32(value, 125..=50_000, "USB period")?;
            if parsed % 125 != 0 {
                return Err(format!("USB period is not a multiple of 125: {parsed}"));
            }
            settings.usb_period = parsed;
        }
        "diagnostic" if matches!(value, "audio" | "bluetooth" | "config" | "alsa") => {
            settings.diagnostic = value.to_string();
        }
        "diagnostic" => return Err(format!("unsupported diagnostic in state: {value}")),
        "diagnostic_all" => settings.diagnostic_all = parse_bool(key, value)?,
        "io_scheduler" if IO_SCHEDULERS.contains(&value) => {
            settings.io_scheduler = value.to_string();
        }
        "io_scheduler" => return Err(format!("unsupported I/O scheduler in state: {value}")),
        "io_tone" if IO_TONES.contains(&value) => settings.io_tone = value.to_string(),
        "io_tone" => return Err(format!("unsupported I/O tone in state: {value}")),
        "wifi_no_restart" => settings.wifi_no_restart = parse_bool(key, value)?,
        "auto_reapply" => settings.auto_reapply = parse_bool(key, value)?,
        key => {
            if let Some(feature) = key
                .strip_prefix("jitter_")
                .and_then(|name| name.strip_suffix("_configured"))
                .filter(|feature| JITTER_FEATURES.contains(feature))
            {
                settings
                    .jitter_configured
                    .insert(feature.to_string(), parse_bool(key, value)?);
            } else if let Some(feature) = key
                .strip_prefix("jitter_")
                .filter(|feature| JITTER_FEATURES.contains(feature))
            {
                settings
                    .jitter_values
                    .insert(feature.to_string(), parse_bool(key, value)?);
            } else {
                return Err(format!("unknown state key: {key}"));
            }
        }
    }
    Ok(())
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("invalid boolean state value for {key}: {value}")),
    }
}

fn validate_resampler_mode(settings: &StoredSettings) -> Result<(), String> {
    let maximum = if settings.resampler_cheat { 200 } else { 100 };
    if (1..=maximum).contains(&settings.resampler_percent) {
        Ok(())
    } else {
        Err(format!(
            "resampler percent out of range for {} mode: {}",
            if settings.resampler_cheat {
                "cheat"
            } else {
                "cutoff"
            },
            settings.resampler_percent
        ))
    }
}

fn normalize_legacy_resampler(settings: &mut StoredSettings, observed: &BTreeSet<&str>) {
    if settings.resampler_preset == "custom"
        && (!RESAMPLER_CUSTOM_KEYS
            .iter()
            .all(|key| observed.contains(key))
            || validate_resampler_mode(settings).is_err())
    {
        reset_resampler_to_canonical_default(settings);
    } else if !settings.resampler_cheat && settings.resampler_percent > 100 {
        settings.resampler_percent = 100;
    }
}

fn reset_resampler_to_canonical_default(settings: &mut StoredSettings) {
    let defaults = StoredSettings::default();
    settings.resampler_preset = defaults.resampler_preset;
    settings.resampler_configured = false;
    settings.resampler_bypass = defaults.resampler_bypass;
    settings.resampler_cheat = defaults.resampler_cheat;
    settings.resampler_stop_band = defaults.resampler_stop_band;
    settings.resampler_half_length = defaults.resampler_half_length;
    settings.resampler_percent = defaults.resampler_percent;
}

fn is_known_state_key(key: &str) -> bool {
    FIXED_STATE_KEYS.contains(&key)
        || key
            .strip_prefix("jitter_")
            .and_then(|name| name.strip_suffix("_configured").or(Some(name)))
            .is_some_and(|name| JITTER_FEATURES.contains(&name))
}

fn required_state_keys() -> Vec<String> {
    let mut keys = FIXED_STATE_KEYS
        .iter()
        .map(|key| (*key).to_string())
        .collect::<Vec<_>>();
    for feature in JITTER_FEATURES {
        keys.push(format!("jitter_{feature}"));
        keys.push(format!("jitter_{feature}_configured"));
    }
    keys
}

fn validate_u32(
    value: &str,
    range: std::ops::RangeInclusive<u32>,
    label: &str,
) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| format!("invalid {label}: {value}"))?;
    if range.contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("{label} out of range: {parsed}"))
    }
}

pub(crate) fn save_policy_settings(policy: &Settings) -> Result<(), String> {
    StateStore::new()
        .update(StateDelta::PolicyApply(policy.clone()))
        .map(|_| ())
}

pub(crate) fn reset_policy_settings() -> Result<(), String> {
    StateStore::new()
        .update(StateDelta::PolicyReset)
        .map(|_| ())
}

pub(crate) fn persist_extra_settings(action: &ExtraAction) -> Result<(), String> {
    StateStore::new()
        .update(StateDelta::Extra(action.clone()))
        .map(|_| ())
}

pub(crate) fn update_stored_settings_for_extra(
    settings: &mut StoredSettings,
    action: &ExtraAction,
) -> bool {
    match action {
        ExtraAction::BluetoothHal { action } if action != "status" => {
            settings.bluetooth_hal = action.clone();
            settings.bluetooth_hal_configured = true;
            true
        }
        ExtraAction::BluetoothHal { .. }
        | ExtraAction::ResamplerStatus
        | ExtraAction::UsbPeriodStatus
        | ExtraAction::JitterStatus => false,
        ExtraAction::ResamplerReset => {
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
        ExtraAction::ResamplerPreset { preset } => {
            settings.resampler_preset = canonical_resampler_preset(preset).to_string();
            settings.resampler_configured = true;
            true
        }
        ExtraAction::ResamplerCustom {
            bypass,
            cheat,
            stop_band,
            half_length,
            percent,
        } => {
            settings.resampler_preset = "custom".to_string();
            settings.resampler_bypass = bypass.clone();
            settings.resampler_cheat = *cheat;
            settings.resampler_stop_band = *stop_band;
            settings.resampler_half_length = *half_length;
            settings.resampler_percent = *percent;
            settings.resampler_configured = true;
            true
        }
        ExtraAction::UsbPeriodReset => {
            settings.usb_period = StoredSettings::default().usb_period;
            settings.usb_period_configured = false;
            true
        }
        ExtraAction::UsbPeriodSet { period } => {
            settings.usb_period = *period;
            settings.usb_period_configured = true;
            true
        }
        ExtraAction::JitterSet {
            enabled,
            feature,
            scheduler,
            tone,
            wifi_no_restart,
        } => {
            if feature == "all" {
                for feature in JITTER_BASE_FEATURES {
                    settings
                        .jitter_values
                        .insert((*feature).to_string(), *enabled);
                    settings
                        .jitter_configured
                        .insert((*feature).to_string(), *enabled);
                }
                return true;
            }
            settings.jitter_values.insert(feature.clone(), *enabled);
            settings.jitter_configured.insert(feature.clone(), true);
            if feature == "io" && *enabled {
                settings.io_scheduler = scheduler.clone().unwrap_or_else(|| "*".to_string());
                settings.io_tone = tone.clone().unwrap_or_else(|| "medium".to_string());
            }
            if feature == "wifi" {
                settings.wifi_no_restart = *enabled && *wifi_no_restart;
            }
            true
        }
        ExtraAction::Diagnose { kind, all } => {
            settings.diagnostic = kind.clone();
            settings.diagnostic_all = *all;
            true
        }
    }
}
