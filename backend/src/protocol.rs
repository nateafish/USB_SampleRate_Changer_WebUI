use std::ffi::OsStr;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::android::{bluetooth_a2dp_state, namespace_info};
use crate::catalog::{
    BIT_DEPTHS, BLUETOOTH_HAL_OPTIONS, DIAGNOSTIC_TYPES, DOCUMENTED_RATES, IO_SCHEDULERS, IO_TONES,
    JITTER_BASE_FEATURES, JITTER_FEATURES, POLICIES, RESAMPLER_BYPASSES, RESAMPLER_MODES,
    RESAMPLER_PRESETS, RESAMPLER_PRESET_GROUPS,
};
use crate::domain::{Settings, StoredSettings};
use crate::paths::{log_root, CORE_DIR, LOG_ROOT};
use crate::state::{bool_number, StateStore};

const CONTROLLER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn print_status(module_dir: &Path) {
    let snapshot = StateStore::new().load();
    println!(
        "state_degraded={}",
        bool_number(snapshot.health.is_degraded())
    );
    if let Some(reason) = snapshot.health.reason() {
        println!(
            "state_degraded_reason={}",
            reason.replace(['\n', '\r'], " ")
        );
    }
    if let Some(version) = snapshot.migrated_from {
        println!("state_migration=v{version}->v3");
    } else if snapshot.recovery_reason.is_some() {
        println!("state_migration=unversioned->v3");
    }
    if let Some(reason) = snapshot.recovery_reason.as_deref() {
        println!(
            "state_recovery_reason={}",
            reason.replace(['\n', '\r'], " ")
        );
    }
    let stored = snapshot.settings;
    let settings = stored.policy.clone();
    let namespace = namespace_info();
    println!("controller_version={CONTROLLER_VERSION}");
    println!("last_command_log={LOG_ROOT}/last-command.log");
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
    if let Ok(last_status) = fs::read_to_string(log_root().join("last.status")) {
        print!("{last_status}");
    }
    print_templates(module_dir);
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
    let a2dp_state = bluetooth_a2dp_state();
    println!("bluetooth_a2dp_state={}", a2dp_state.label());
    // Keep the historical boolean for older WebUI clients; unknown is never
    // reported as connected.
    println!(
        "bluetooth_a2dp_connected={}",
        bool_number(matches!(a2dp_state, crate::android::A2dpState::Connected))
    );
    println!("amzm={}", bool_number(settings.amzm));
    println!("test={}", bool_number(settings.test));
    println!(
        "test_template={}",
        settings.test_template.as_deref().unwrap_or("")
    );
}

pub(crate) fn print_schema(module_dir: &Path) {
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

/// Render the machine-readable controller contract.  This deliberately uses
/// a small, dependency-free JSON writer: the Android release build has no
/// third-party crates, and all values originate from our static catalog or a
/// trusted module path.  `json_quote` still escapes every string so the
/// output remains valid if a future catalog entry contains punctuation.
pub(crate) fn print_schema_json(module_dir: &Path) {
    println!("{}", render_schema_json(module_dir));
}

#[cfg(test)]
pub(crate) fn render_schema_json_for_test(module_dir: &Path) -> String {
    render_schema_json(module_dir)
}

fn render_schema_json(module_dir: &Path) -> String {
    let mut out = String::with_capacity(16 * 1024);
    out.push('{');
    json_number(&mut out, "schema_version", 1);
    out.push(',');
    json_number(&mut out, "api_version", 1);
    out.push(',');
    json_string_field(&mut out, "controller_version", CONTROLLER_VERSION);
    out.push(',');
    json_string_field(
        &mut out,
        "script_version",
        &upstream_script_version(module_dir),
    );
    out.push_str(",\"capabilities\":{");
    out.push_str("\"json_schema\":true,\"audio_restart_interface\":true,\"batch_reapply\":true}");

    out.push_str(",\"limits\":{");
    out.push_str("\"sample_rate\":{\"min\":44100,\"max\":768000,\"integer\":true}");
    out.push_str(",\"usb_period\":{\"min\":125,\"max\":50000,\"step\":125,\"unit\":\"usec\"}");
    out.push_str(",\"resampler\":{");
    out.push_str("\"stop_band\":{\"min\":20,\"max\":242,\"step\":1}");
    out.push_str(",\"half_length\":{\"min\":8,\"max\":640,\"step\":8}");
    out.push_str(",\"cutoff_percent\":{\"min\":1,\"max\":100,\"step\":1}");
    out.push_str(",\"cheat_percent\":{\"min\":1,\"max\":200,\"step\":1}}");
    out.push('}');

    out.push_str(",\"policy\":{\"default\":\"auto\",\"options\":[");
    for (index, (value, flag, _label)) in POLICIES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(&mut out, "flag", flag);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("policy.option.{value}.label"),
        );
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("policy.option.{value}.description"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "auto" { "true" } else { "false" });
        out.push_str(",\"recommended\":");
        out.push_str(if *value == "auto" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("]}");

    out.push_str(",\"sample_rates\":[");
    for (index, (value, _label)) in DOCUMENTED_RATES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_number(&mut out, "value", *value);
        out.push(',');
        json_string_field(&mut out, "label_key", &format!("rate.{value}"));
        out.push_str(",\"default\":");
        out.push_str(if *value == 44_100 { "true" } else { "false" });
        out.push('}');
    }
    out.push(']');

    out.push_str(",\"bit_depths\":[");
    for (index, (value, _label)) in BIT_DEPTHS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(&mut out, "label_key", &format!("format.{value}"));
        out.push_str(",\"default\":");
        out.push_str(if *value == "32" { "true" } else { "false" });
        out.push('}');
    }
    out.push(']');

    out.push_str(",\"switches\":[");
    for (index, (key, flag)) in [
        ("drc", "--drc"),
        ("force_usbv2", "--force-usbv2"),
        ("force_bluetooth_qti", "--force-bluetooth-qti"),
        ("amzm", "--amzm"),
        ("test", "--test"),
    ]
    .iter()
    .enumerate()
    {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "key", key);
        out.push(',');
        json_string_field(&mut out, "flag", flag);
        out.push_str(",\"default\":false,");
        json_string_field(&mut out, "label_key", &format!("switch.{key}.label"));
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("switch.{key}.description"),
        );
        out.push('}');
    }
    out.push(']');

    out.push_str(",\"extras\":{");
    out.push_str("\"bluetooth_hal\":{");
    out.push_str("\"available\":true,\"tool\":\"bluetooth-hal\",");
    json_string_field(&mut out, "label_key", "tools.bluetooth_hal.label");
    out.push(',');
    json_string_field(
        &mut out,
        "description_key",
        "tools.bluetooth_hal.description",
    );
    out.push_str(",\"default\":\"offload\",\"recommended\":\"offload\"");
    out.push_str(",\"actions\":[");
    for (index, value) in ["status", "aosp", "legacy", "offload", "sysbta"]
        .iter()
        .enumerate()
    {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("bluetooth_hal.action.{value}.label"),
        );
        out.push_str(",\"kind\":");
        out.push_str(&json_quote(if *value == "status" {
            "status"
        } else {
            "set"
        }));
        out.push_str(",\"selectable\":");
        out.push_str(if *value == "status" { "false" } else { "true" });
        out.push('}');
    }
    out.push_str("],\"options\":[");
    for (index, value) in BLUETOOTH_HAL_OPTIONS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("bluetooth_hal.option.{value}.label"),
        );
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("bluetooth_hal.option.{value}.description"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "offload" { "true" } else { "false" });
        out.push_str(",\"recommended\":");
        out.push_str(if *value == "offload" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("],\"operations\":{\"status\":true,\"set\":true,\"reset\":false}}");

    out.push_str(",\"resampler\":{");
    out.push_str("\"available\":true,\"tool\":\"resampler\",");
    json_string_field(&mut out, "label_key", "tools.resampler.label");
    out.push(',');
    json_string_field(&mut out, "description_key", "tools.resampler.description");
    out.push_str(",\"default_preset\":\"179-408-99\",\"upstream_default_preset\":\"default\",\"recommended\":\"179-408-99\"");
    out.push_str(",\"actions\":[");
    json_action_array(
        &mut out,
        &[
            ("status", "status", false),
            ("reset", "reset", false),
            ("preset", "set_preset", true),
            ("custom", "set_custom", true),
        ],
    );
    out.push(']');
    out.push_str(",\"presets\":[");
    for (index, (value, _args)) in RESAMPLER_PRESETS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("resampler.preset.{value}.label"),
        );
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("resampler.preset.{value}.description"),
        );
        out.push_str(",\"kind\":\"preset\"");
        out.push_str(",\"selectable\":");
        out.push_str("true");
        out.push_str(",\"default\":");
        out.push_str(if *value == "179-408-99" {
            "true"
        } else {
            "false"
        });
        out.push_str(",\"recommended\":");
        out.push_str(if *value == "179-408-99" {
            "true"
        } else {
            "false"
        });
        out.push('}');
    }
    out.push_str("],\"groups\":[");
    for (index, (value, options)) in RESAMPLER_PRESET_GROUPS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("resampler.group.{value}.label"),
        );
        out.push_str(",\"options\":[");
        json_string_array(&mut out, options);
        out.push_str("]}");
    }
    out.push_str("],\"custom\":{");
    json_string_field(&mut out, "value", "custom");
    out.push(',');
    json_string_field(&mut out, "label_key", "resampler.custom.label");
    out.push(',');
    json_string_field(&mut out, "description_key", "resampler.custom.description");
    out.push_str(",\"default\":{\"bypass\":\"none\",\"mode\":\"cheat\",\"stop_band\":179,\"half_length\":408,\"percent\":99}");
    out.push_str(",\"bypass_options\":[");
    for (index, (value, sample_rate)) in RESAMPLER_BYPASSES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("resampler.bypass.{value}.label"),
        );
        out.push(',');
        json_number(&mut out, "sample_rate", *sample_rate);
        out.push_str(",\"default\":");
        out.push_str(if *value == "none" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("],\"modes\":[");
    for (index, value) in RESAMPLER_MODES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("resampler.mode.{value}.label"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "cheat" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("],\"parameters\":{");
    out.push_str(
        "\"stop_band\":{\"min\":20,\"max\":242,\"step\":1,\"default\":179,\"unit\":\"dB\"}",
    );
    out.push_str(",\"half_length\":{\"min\":8,\"max\":640,\"step\":8,\"default\":408}");
    out.push_str(",\"cutoff_percent\":{\"min\":1,\"max\":100,\"step\":1,\"default\":99,\"unit\":\"percent\"}");
    out.push_str(",\"cheat_percent\":{\"min\":1,\"max\":200,\"step\":1,\"default\":99,\"unit\":\"percent\"}}");
    out.push_str("},\"operations\":{\"status\":true,\"set_preset\":true,\"set_custom\":true,\"reset\":true}}");

    out.push_str(",\"usb_period\":{");
    out.push_str("\"available\":true,\"tool\":\"usb-period\",");
    json_string_field(&mut out, "label_key", "tools.usb_period.label");
    out.push(',');
    json_string_field(&mut out, "description_key", "tools.usb_period.description");
    out.push_str(",\"default\":2250,\"recommended\":2250");
    out.push_str(",\"range\":{\"min\":125,\"max\":50000,\"step\":125,\"unit\":\"usec\"}");
    out.push_str(",\"actions\":[");
    json_action_array(
        &mut out,
        &[
            ("status", "status", false),
            ("set", "set", true),
            ("reset", "reset", false),
        ],
    );
    out.push(']');
    out.push_str(",\"operations\":{\"status\":true,\"set\":true,\"reset\":true}}");
    out.push_str(",\"jitter\":{");
    out.push_str("\"available\":true,\"tool\":\"jitter\",");
    json_string_field(&mut out, "label_key", "tools.jitter.label");
    out.push(',');
    json_string_field(&mut out, "description_key", "tools.jitter.description");
    out.push_str(",\"actions\":[");
    json_action_array(
        &mut out,
        &[
            ("status", "status", false),
            ("enable", "set", true),
            ("disable", "set", true),
            ("all", "set_all", true),
            ("reset", "reset", false),
        ],
    );
    out.push(']');
    out.push_str(",\"features\":[");
    for (index, feature) in JITTER_FEATURES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", feature);
        out.push(',');
        json_string_field(&mut out, "label_key", &format!("jitter.{feature}.label"));
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("jitter.{feature}.description"),
        );
        out.push_str(",\"default\":false,\"high_risk\":");
        out.push_str(if matches!(*feature, "selinux" | "thermal") {
            "true"
        } else {
            "false"
        });
        out.push_str(",\"requires_audio_restart\":");
        out.push_str(if *feature == "effect" {
            "true"
        } else {
            "false"
        });
        out.push_str(",\"capabilities\":{");
        out.push_str("\"io_parameters\":");
        out.push_str(if *feature == "io" { "true" } else { "false" });
        out.push_str(",\"wifi_no_restart\":");
        out.push_str(if *feature == "wifi" { "true" } else { "false" });
        out.push('}');
        out.push('}');
    }
    out.push_str("],\"io_schedulers\":[");
    json_string_array(&mut out, IO_SCHEDULERS);
    out.push_str("],\"io_tones\":[");
    json_string_array(&mut out, IO_TONES);
    out.push_str("],\"io_scheduler_options\":[");
    for (index, value) in IO_SCHEDULERS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("jitter.io_scheduler.{value}.label"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "*" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("],\"io_tone_options\":[");
    for (index, value) in IO_TONES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(
            &mut out,
            "label_key",
            &format!("jitter.io_tone.{value}.label"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "medium" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str(
        "],\"defaults\":{\"io_scheduler\":\"*\",\"io_tone\":\"medium\",\"wifi_no_restart\":false}",
    );
    out.push_str(",\"wifi_no_restart\":{\"supported\":true,\"default\":false,\"label_key\":\"jitter.wifi.no_restart.label\",\"description_key\":\"jitter.wifi.no_restart.description\"}");
    out.push_str(",\"reset_features\":[");
    json_string_array(&mut out, JITTER_BASE_FEATURES);
    out.push_str("],\"operations\":{\"status\":true,\"set\":true,\"reset\":true}}");

    out.push_str(",\"diagnostics\":{");
    out.push_str("\"available\":true,\"tool\":\"diagnose\",");
    json_string_field(&mut out, "label_key", "tools.diagnostics.label");
    out.push(',');
    json_string_field(&mut out, "description_key", "tools.diagnostics.description");
    out.push_str(",\"default\":\"audio\"");
    out.push_str(",\"actions\":[");
    json_action_array(&mut out, &[("run", "run", true)]);
    out.push(']');
    out.push_str(",\"types\":[");
    for (index, value) in DIAGNOSTIC_TYPES.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(&mut out, "value", value);
        out.push(',');
        json_string_field(&mut out, "label_key", &format!("diagnostic.{value}.label"));
        out.push(',');
        json_string_field(
            &mut out,
            "description_key",
            &format!("diagnostic.{value}.description"),
        );
        out.push_str(",\"default\":");
        out.push_str(if *value == "audio" { "true" } else { "false" });
        out.push('}');
    }
    out.push_str("],\"complete_output\":{\"supported\":true,\"argument\":\"all\",\"default\":false,\"label_key\":\"diagnostics.complete_output.label\",\"description_key\":\"diagnostics.complete_output.description\"}");
    out.push_str(",\"operations\":{\"run\":true,\"status\":false,\"reset\":false}}}");

    out.push_str(",\"templates\":[");
    let templates = collect_templates(&module_dir.join(CORE_DIR).join("templates"));
    json_string_array(
        &mut out,
        &templates.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    out.push_str("]}");
    out
}

fn json_string_array(out: &mut String, values: &[&str]) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_quote(value));
    }
}

fn json_action_array(out: &mut String, actions: &[(&str, &str, bool)]) {
    for (index, (value, kind, selectable)) in actions.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('{');
        json_string_field(out, "value", value);
        out.push(',');
        json_string_field(out, "kind", kind);
        out.push_str(",\"selectable\":");
        out.push_str(if *selectable { "true" } else { "false" });
        out.push('}');
    }
}

fn json_string_field(out: &mut String, key: &str, value: &str) {
    out.push_str(&json_quote(key));
    out.push(':');
    out.push_str(&json_quote(value));
}

fn json_number(out: &mut String, key: &str, value: u32) {
    out.push_str(&json_quote(key));
    out.push(':');
    let _ = write!(out, "{value}");
}

fn json_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

fn print_templates(module_dir: &Path) {
    println!("templates_begin");
    for template in collect_templates(&module_dir.join(CORE_DIR).join("templates")) {
        println!("template={template}");
    }
    println!("templates_end");
}

pub(crate) fn collect_templates(root: &Path) -> Vec<String> {
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

pub(crate) fn print_logs() -> Result<(), String> {
    let path = log_root().join("last.log");
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

pub(crate) fn print_generated() -> Result<(), String> {
    let path: PathBuf = log_root().join("last-command.log");
    if !path.exists() {
        return Err("no last command log exists".to_string());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read last command log: {error}"))?;
    print!("{content}");
    Ok(())
}
