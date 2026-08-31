use crate::catalog::JITTER_FEATURES;
use crate::domain::{ExtraAction, ReapplyAction, StoredSettings};

pub(crate) fn build_reapply_plan(stored: &StoredSettings) -> Vec<ReapplyAction> {
    if !stored.auto_reapply {
        return Vec::new();
    }
    let mut plan = Vec::new();
    if stored.bluetooth_hal_configured {
        plan.push(ReapplyAction::Extra(ExtraAction::BluetoothHal {
            action: stored.bluetooth_hal.clone(),
        }));
    }
    if stored.policy_configured {
        plan.push(ReapplyAction::Policy(stored.policy.clone()));
    }
    if stored.resampler_configured {
        let action = if stored.resampler_preset == "custom" {
            ExtraAction::ResamplerCustom {
                bypass: stored.resampler_bypass.clone(),
                cheat: stored.resampler_cheat,
                stop_band: stored.resampler_stop_band,
                half_length: stored.resampler_half_length,
                percent: stored.resampler_percent,
            }
        } else {
            ExtraAction::ResamplerPreset {
                preset: stored.resampler_preset.clone(),
            }
        };
        plan.push(ReapplyAction::Extra(action));
    }
    if stored.usb_period_configured {
        plan.push(ReapplyAction::Extra(ExtraAction::UsbPeriodSet {
            period: stored.usb_period,
        }));
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
        plan.push(ReapplyAction::Extra(ExtraAction::JitterSet {
            enabled,
            feature: (*feature).to_string(),
            scheduler: (*feature == "io" && enabled).then(|| stored.io_scheduler.clone()),
            tone: (*feature == "io" && enabled).then(|| stored.io_tone.clone()),
            wifi_no_restart: *feature == "wifi" && enabled && stored.wifi_no_restart,
        }));
    }
    plan
}
