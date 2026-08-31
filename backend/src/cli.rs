use crate::catalog::{
    canonical_resampler_preset, resampler_preset_args, BLUETOOTH_HAL_OPTIONS, DIAGNOSTIC_TYPES,
    IO_SCHEDULERS, IO_TONES, JITTER_FEATURES, RESAMPLER_BYPASSES, RESAMPLER_MODES,
};
use crate::domain::{ExtraAction, Settings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ControllerCommand {
    Schema { json: bool },
    Status,
    Logs,
    Generated,
    Preview(Settings),
    Apply(Settings),
    Reset,
    Extra(ExtraAction),
    SetAutoReapply(bool),
    Reapply,
}

pub(crate) fn parse(args: &[String]) -> Result<ControllerCommand, String> {
    match args.get(1).map(String::as_str) {
        Some("schema") if args.len() == 2 => Ok(ControllerCommand::Schema { json: false }),
        Some("schema") if args.len() == 3 && args[2] == "--json" => {
            Ok(ControllerCommand::Schema { json: true })
        }
        Some("status") if args.len() == 2 => Ok(ControllerCommand::Status),
        Some("logs") if args.len() == 2 => Ok(ControllerCommand::Logs),
        Some("generated") if args.len() == 2 => Ok(ControllerCommand::Generated),
        Some("preview") => Ok(ControllerCommand::Preview(parse_settings_args(&args[2..])?)),
        Some("apply") => Ok(ControllerCommand::Apply(parse_settings_args(&args[2..])?)),
        Some("reset") if args.len() == 2 => Ok(ControllerCommand::Reset),
        Some("extra") => Ok(ControllerCommand::Extra(parse_extra_action(&args[2..])?)),
        Some("settings") => parse_settings_command(&args[2..]),
        Some("reapply") if args.len() == 2 => Ok(ControllerCommand::Reapply),
        _ => Err(format!(
            "usage: {} {{schema [--json]|status|logs|generated|preview OPTIONS|apply OPTIONS|reset|extra TOOL ACTION|settings auto-reapply enable|disable|reapply}}",
            args.first().map(String::as_str).unwrap_or("usbsrctl")
        )),
    }
}

fn parse_settings_command(args: &[String]) -> Result<ControllerCommand, String> {
    if args.len() != 2 || args[0] != "auto-reapply" {
        return Err("settings usage: settings auto-reapply enable|disable".to_string());
    }
    match args[1].as_str() {
        "enable" => Ok(ControllerCommand::SetAutoReapply(true)),
        "disable" => Ok(ControllerCommand::SetAutoReapply(false)),
        _ => Err("auto-reapply must be enable or disable".to_string()),
    }
}

pub(crate) fn parse_settings_args(args: &[String]) -> Result<Settings, String> {
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

pub(crate) fn normalize_sample_rate(value: &str) -> Result<u32, String> {
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

pub(crate) fn parse_extra_action(args: &[String]) -> Result<ExtraAction, String> {
    let tool = args.first().map(String::as_str).ok_or_else(extra_usage)?;
    let action = args.get(1).map(String::as_str).ok_or_else(extra_usage)?;
    match tool {
        "bluetooth-hal" => {
            if args.len() != 2 || (action != "status" && !BLUETOOTH_HAL_OPTIONS.contains(&action)) {
                return Err(
                    "bluetooth-hal action must be status, aosp, legacy, offload, or sysbta"
                        .to_string(),
                );
            }
            Ok(ExtraAction::BluetoothHal {
                action: action.to_string(),
            })
        }
        "resampler" => parse_resampler_action(args),
        "usb-period" => {
            if args.len() != 2 {
                return Err(
                    "usb-period action must be status, reset, or a period in usec".to_string(),
                );
            }
            match action {
                "status" => Ok(ExtraAction::UsbPeriodStatus),
                "reset" => Ok(ExtraAction::UsbPeriodReset),
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
                    Ok(ExtraAction::UsbPeriodSet { period })
                }
            }
        }
        "jitter" => parse_jitter_action(args),
        "diagnose" => parse_diagnostic_action(args),
        _ => Err(extra_usage()),
    }
}

fn parse_resampler_action(args: &[String]) -> Result<ExtraAction, String> {
    let action = args[1].as_str();
    if action == "custom" {
        return parse_custom_resampler(args);
    }
    if args.len() != 2 {
        return Err(
            "resampler action must be status, reset, a preset name, or custom parameters"
                .to_string(),
        );
    }
    match action {
        "status" => Ok(ExtraAction::ResamplerStatus),
        "reset" => Ok(ExtraAction::ResamplerReset),
        preset if resampler_preset_args(preset).is_some() => Ok(ExtraAction::ResamplerPreset {
            preset: canonical_resampler_preset(preset).to_string(),
        }),
        preset => Err(format!("unsupported resampler preset: {preset}")),
    }
}

fn parse_jitter_action(args: &[String]) -> Result<ExtraAction, String> {
    if args.get(1).map(String::as_str) == Some("status") && args.len() == 2 {
        return Ok(ExtraAction::JitterStatus);
    }
    if args.len() < 3 || !matches!(args[1].as_str(), "enable" | "disable") {
        return Err(
            "jitter usage: jitter status | jitter enable|disable FEATURE [SCHEDULER TONE]"
                .to_string(),
        );
    }
    let enabled = args[1] == "enable";
    let feature = args[2].as_str();
    if feature != "all" && !JITTER_FEATURES.contains(&feature) {
        return Err(format!("unsupported jitter feature: {feature}"));
    }

    let mut scheduler = None;
    let mut tone = None;
    let mut wifi_no_restart = false;
    if feature == "io" && enabled {
        let selected_scheduler = args.get(3).map(String::as_str).unwrap_or("*");
        let selected_tone = args.get(4).map(String::as_str).unwrap_or("medium");
        if !IO_SCHEDULERS.contains(&selected_scheduler)
            || !IO_TONES.contains(&selected_tone)
            || args.len() > 5
        {
            return Err("unsupported I/O scheduler or tone".to_string());
        }
        scheduler = Some(selected_scheduler.to_string());
        tone = Some(selected_tone.to_string());
    } else if feature == "wifi" && enabled && args.len() == 4 && args[3] == "no-restart" {
        wifi_no_restart = true;
    } else if args.len() != 3 {
        return Err("only enabled I/O accepts scheduler and tone arguments".to_string());
    }

    Ok(ExtraAction::JitterSet {
        enabled,
        feature: feature.to_string(),
        scheduler,
        tone,
        wifi_no_restart,
    })
}

fn parse_custom_resampler(args: &[String]) -> Result<ExtraAction, String> {
    if args.len() != 7 {
        return Err("custom resampler usage: resampler custom none|48|96 cutoff|cheat STOP_DB HALF_LENGTH PERCENT".to_string());
    }
    let bypass = args[2].as_str();
    let mode = args[3].as_str();
    if !RESAMPLER_BYPASSES.iter().any(|(value, _)| *value == bypass)
        || !RESAMPLER_MODES.contains(&mode)
    {
        return Err("invalid resampler bypass or cutoff mode".to_string());
    }
    let stop_band = args[4]
        .parse::<u32>()
        .map_err(|_| "invalid stop-band value".to_string())?;
    let half_length = args[5]
        .parse::<u32>()
        .map_err(|_| "invalid half-filter length".to_string())?;
    let percent = args[6]
        .parse::<u32>()
        .map_err(|_| "invalid cutoff/cheat percent".to_string())?;
    if !(20..=242).contains(&stop_band)
        || !(8..=640).contains(&half_length)
        || half_length % 8 != 0
        || (mode == "cutoff" && !(1..=100).contains(&percent))
        || (mode == "cheat" && !(1..=200).contains(&percent))
    {
        return Err("custom resampler parameters are outside upstream ranges".to_string());
    }
    Ok(ExtraAction::ResamplerCustom {
        bypass: bypass.to_string(),
        cheat: mode == "cheat",
        stop_band,
        half_length,
        percent,
    })
}

fn parse_diagnostic_action(args: &[String]) -> Result<ExtraAction, String> {
    if !(2..=3).contains(&args.len()) {
        return Err("diagnose usage: diagnose audio|bluetooth|config|alsa [all]".to_string());
    }
    let kind = if DIAGNOSTIC_TYPES.contains(&args[1].as_str()) {
        args[1].clone()
    } else {
        return Err(format!("unsupported diagnostic: {}", args[1]));
    };
    let all = if args.len() == 2 {
        false
    } else if args[2] == "all" {
        true
    } else {
        return Err("diagnostic detail must be all".to_string());
    };
    Ok(ExtraAction::Diagnose { kind, all })
}

fn extra_usage() -> String {
    "extra tool must be bluetooth-hal, resampler, usb-period, jitter, or diagnose".to_string()
}
