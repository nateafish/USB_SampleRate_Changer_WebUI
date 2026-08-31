use std::collections::BTreeSet;
use std::path::Path;

use crate::catalog::{BIT_DEPTHS, POLICIES};
use crate::domain::{Action, ExtraAction, ReapplyAction, Settings};
use crate::paths::CORE_DIR;

const GUARDED_SHELL_PREFIX: &str = "#!/system/bin/sh\n\
set -u\n\
audio_pid=\"$(pidof audioserver 2>/dev/null)\"\n\
audio_pid=\"${audio_pid%% *}\"\n\
if [ -z \"$audio_pid\" ]; then\n\
    audio_pid=\"$(getprop init.svc_debug_pid.audioserver 2>/dev/null)\"\n\
fi\n\
[ -n \"$audio_pid\" ] || { echo \"audioserver is not running\" >&2; exit 70; }\n\
self_ns=\"$(readlink /proc/self/ns/mnt 2>/dev/null)\"\n\
audio_ns=\"$(readlink \"/proc/$audio_pid/ns/mnt\" 2>/dev/null)\"\n\
if [ -z \"$self_ns\" ] || [ -z \"$audio_ns\" ] || [ \"$self_ns\" != \"$audio_ns\" ]; then\n\
    echo \"mount namespace mismatch: self=$self_ns audio=$audio_ns\" >&2\n\
    exit 71\n\
fi\n\
echo \"namespace verified: $self_ns\"\n";

pub(crate) fn validate_settings(settings: &Settings, module_dir: &Path) -> Result<(), String> {
    if policy_flag(&settings.policy).is_none() {
        return Err(format!("unsupported policy mode: {}", settings.policy));
    }
    crate::cli::normalize_sample_rate(&settings.sample_rate.to_string())?;
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
            );
        }
        (false, Some(_)) => return Err("--test-template requires --test".to_string()),
        (false, None) => {}
    }
    validate_upstream_script(module_dir)
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

pub(crate) fn validate_settings_for_action(
    settings: &Settings,
    module_dir: &Path,
    action: Action,
) -> Result<(), String> {
    match action {
        Action::Apply => validate_settings(settings, module_dir),
        Action::Reset => validate_upstream_script(module_dir),
    }
}

fn validate_upstream_script(module_dir: &Path) -> Result<(), String> {
    let script = module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh");
    if script.is_file() {
        Ok(())
    } else {
        Err(format!("upstream script not found: {}", script.display()))
    }
}

fn policy_flag(policy: &str) -> Option<&'static str> {
    POLICIES
        .iter()
        .find(|(value, _, _)| *value == policy)
        .map(|(_, flag, _)| *flag)
}

pub(crate) fn upstream_args(settings: &Settings, action: Action) -> Vec<String> {
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

pub(crate) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn command_for(script: &Path, args: impl IntoIterator<Item = String>) -> String {
    std::iter::once("/system/bin/sh".to_string())
        .chain(std::iter::once(script.to_string_lossy().into_owned()))
        .chain(args)
        .map(|argument| shell_quote(&argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the concrete upstream command represented by a policy operation.
/// This is used for diagnostics; execution still happens through the guarded
/// in-memory wrapper sent to `sh -s`.
pub(crate) fn policy_command_summary(
    settings: &Settings,
    module_dir: &Path,
    action: Action,
) -> String {
    command_for(
        &module_dir.join(CORE_DIR).join("USB_SampleRate_Changer.sh"),
        upstream_args(settings, action),
    )
}

pub(crate) fn extra_command_summary(script: &Path, action: &ExtraAction) -> String {
    command_for(script, action.script_args())
}

pub(crate) fn extra_command_summaries(script: &Path, action: &ExtraAction) -> Vec<String> {
    let mut commands = vec![extra_command_summary(script, action)];
    if let Some(restart) = extra_restart_command(script, action) {
        commands.push(restart);
        if let Some(status) = extra_status_command(script, action) {
            commands.push(status);
        }
    }
    commands
}

pub(crate) struct ReapplyScriptStep {
    pub(crate) label: String,
    pub(crate) script: String,
}

/// Return reapply's ordered upstream commands, including the coalesced audio
/// restart as the final step when required.
pub(crate) fn reapply_command_summaries(plan: &[ReapplyAction], module_dir: &Path) -> Vec<String> {
    let mut commands = Vec::with_capacity(plan.len() + 1);
    for action in plan {
        match action {
            ReapplyAction::Policy(settings) => {
                commands.push(policy_command_summary(settings, module_dir, Action::Apply));
            }
            ReapplyAction::Extra(action) => {
                commands.push(extra_command_summary(
                    &module_dir
                        .join(CORE_DIR)
                        .join("extras")
                        .join(action.script()),
                    action,
                ));
            }
        }
    }
    if plan.iter().any(ReapplyAction::requires_audio_restart) {
        let reload = module_dir
            .join(CORE_DIR)
            .join("extras")
            .join("reload-audio-servers.sh");
        let mut args = vec![
            "/system/bin/sh".to_string(),
            reload.to_string_lossy().into_owned(),
        ];
        if let Some(mode) = plan.iter().find_map(|action| match action {
            ReapplyAction::Extra(ExtraAction::BluetoothHal { action }) if action != "status" => {
                Some(action.as_str())
            }
            _ => None,
        }) {
            args.extend(["--bluetooth-hal".to_string(), mode.to_string()]);
        }
        commands.push(
            args.into_iter()
                .map(|argument| shell_quote(&argument))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mut seen = BTreeSet::new();
        for action in plan {
            let ReapplyAction::Extra(extra) = action else {
                continue;
            };
            if !extra.requires_audio_restart() {
                continue;
            }
            let script = module_dir
                .join(CORE_DIR)
                .join("extras")
                .join(extra.script());
            if let Some(status) = extra_status_command(&script, extra) {
                if seen.insert(status.clone()) {
                    commands.push(status);
                }
            }
        }
    }
    commands
}

pub(crate) fn render_policy_script(
    settings: &Settings,
    module_dir: &Path,
    action: Action,
) -> String {
    let command = policy_command_summary(settings, module_dir, action);
    let restart = restart_command(module_dir, action);

    format!(
        "{GUARDED_SHELL_PREFIX}echo 'controller_upstream_started=1' >&2\n{command}\nstatus=$?\nif [ $status -eq 0 ]; then exec {restart}; else exit $status; fi\n"
    )
}

pub(crate) fn render_extra_script(script: &Path, action: &ExtraAction) -> String {
    let command = extra_command_summary(script, action);
    format!("{GUARDED_SHELL_PREFIX}echo 'controller_upstream_started=1' >&2\nexec {command}\n")
}

pub(crate) fn render_extra_restart_script(script: &Path, action: &ExtraAction) -> Option<String> {
    extra_restart_command(script, action).map(|command| {
        format!("{GUARDED_SHELL_PREFIX}echo 'controller_restart_started=1' >&2\nexec {command}\n")
    })
}

pub(crate) fn render_extra_status_script(script: &Path, action: &ExtraAction) -> Option<String> {
    extra_status_command(script, action).map(|command| {
        format!(
            "{GUARDED_SHELL_PREFIX}echo 'controller_verification_started=1' >&2\nexec {command}\n"
        )
    })
}

fn restart_command(module_dir: &Path, action: Action) -> String {
    let reload = module_dir
        .join(CORE_DIR)
        .join("extras")
        .join("reload-audio-servers.sh");
    let mut args = vec![
        "/system/bin/sh".to_string(),
        reload.to_string_lossy().into_owned(),
    ];
    if matches!(action, Action::Reset) {
        args.push("--all".to_string());
    }
    args.iter()
        .map(String::as_str)
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn extra_restart_command(script: &Path, action: &ExtraAction) -> Option<String> {
    if !action.requires_audio_restart() {
        return None;
    }
    let reload = script
        .parent()
        .unwrap_or(script)
        .join("reload-audio-servers.sh");
    let mut args = vec![
        "/system/bin/sh".to_string(),
        reload.to_string_lossy().into_owned(),
    ];
    if let ExtraAction::BluetoothHal { action } = action {
        args.extend(["--bluetooth-hal".to_string(), action.clone()]);
    }
    Some(
        args.iter()
            .map(String::as_str)
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn extra_status_command(script: &Path, action: &ExtraAction) -> Option<String> {
    let args = match action {
        ExtraAction::BluetoothHal { .. }
        | ExtraAction::ResamplerReset
        | ExtraAction::ResamplerPreset { .. }
        | ExtraAction::ResamplerCustom { .. }
        | ExtraAction::UsbPeriodReset
        | ExtraAction::UsbPeriodSet { .. }
        | ExtraAction::JitterSet { .. } => vec!["--status".to_string()],
        _ => return None,
    };
    Some(command_for(script, args))
}

pub(crate) fn validate_reapply_plan(
    plan: &[ReapplyAction],
    module_dir: &Path,
) -> Result<(), String> {
    for action in plan {
        match action {
            ReapplyAction::Policy(settings) => validate_settings(settings, module_dir)?,
            ReapplyAction::Extra(action) => {
                let script = module_dir
                    .join(CORE_DIR)
                    .join("extras")
                    .join(action.script());
                if !script.is_file() {
                    return Err(format!("extras script not found: {}", script.display()));
                }
            }
        }
    }
    if plan.iter().any(ReapplyAction::requires_audio_restart) {
        let reload = module_dir
            .join(CORE_DIR)
            .join("extras")
            .join("reload-audio-servers.sh");
        if !reload.is_file() {
            return Err(format!(
                "audio reload script not found: {}",
                reload.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn render_reapply_steps(
    plan: &[ReapplyAction],
    module_dir: &Path,
) -> Vec<ReapplyScriptStep> {
    plan.iter()
        .map(|action| {
            let (label, command) = match action {
                ReapplyAction::Policy(settings) => {
                    let command = policy_command_summary(settings, module_dir, Action::Apply);
                    ("policy".to_string(), command)
                }
                ReapplyAction::Extra(action) => {
                    let upstream = module_dir
                        .join(CORE_DIR)
                        .join("extras")
                        .join(action.script());
                    let command = extra_command_summary(&upstream, action);
                    (action.tool(), command)
                }
            };
            let script = format!(
                "{GUARDED_SHELL_PREFIX}echo 'controller_upstream_started=1' >&2\nexec {command}\n"
            );
            ReapplyScriptStep { label, script }
        })
        .collect()
}

pub(crate) fn render_reapply_restart_script(
    plan: &[ReapplyAction],
    module_dir: &Path,
) -> Option<String> {
    if !plan.iter().any(ReapplyAction::requires_audio_restart) {
        return None;
    }
    let bluetooth_mode = plan.iter().find_map(|action| match action {
        ReapplyAction::Extra(ExtraAction::BluetoothHal { action }) if action != "status" => {
            Some(action.as_str())
        }
        _ => None,
    });
    let reload = module_dir
        .join(CORE_DIR)
        .join("extras")
        .join("reload-audio-servers.sh");
    let mut reload_args = vec![
        "/system/bin/sh".to_string(),
        reload.to_string_lossy().into_owned(),
    ];
    if let Some(mode) = bluetooth_mode {
        reload_args.extend(["--bluetooth-hal".to_string(), mode.to_string()]);
    }
    let command = reload_args
        .iter()
        .map(String::as_str)
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "{GUARDED_SHELL_PREFIX}echo 'controller_upstream_started=1' >&2\nexec {command}\n"
    ))
}

/// Build independent, de-duplicated status queries for Extras whose settings
/// only become authoritative after the coalesced audio restart. Policy has no
/// reliable status command and is intentionally excluded.
pub(crate) fn render_reapply_verifications(
    plan: &[ReapplyAction],
    module_dir: &Path,
) -> Vec<ReapplyScriptStep> {
    let mut seen = BTreeSet::new();
    let mut verifications = Vec::new();
    for action in plan {
        let ReapplyAction::Extra(extra) = action else {
            continue;
        };
        if !extra.requires_audio_restart() {
            continue;
        }
        let upstream = module_dir
            .join(CORE_DIR)
            .join("extras")
            .join(extra.script());
        let Some(command) = extra_status_command(&upstream, extra) else {
            continue;
        };
        if !seen.insert(command.clone()) {
            continue;
        }
        verifications.push(ReapplyScriptStep {
            label: extra.tool(),
            script: format!(
                "{GUARDED_SHELL_PREFIX}echo 'controller_verification_started=1' >&2\nexec {command}\n"
            ),
        });
    }
    verifications
}

#[cfg(test)]
pub(crate) fn render_reapply_script(plan: &[ReapplyAction], module_dir: &Path) -> String {
    let steps = render_reapply_steps(plan, module_dir);
    let mut script = String::from("batch_status=0\n");
    for (index, step) in steps.iter().enumerate() {
        let number = index + 1;
        script.push_str(&format!(
            "echo 'batch_step={number}:{}'\necho 'batch_step_started={number}'\n",
            step.label
        ));
        script.push_str(&step.script);
        script.push_str(&format!(
            "echo 'batch_step_exit={number}:$?'\necho \"batch_step_exit={number}:$step_status\"\n"
        ));
    }
    if render_reapply_restart_script(plan, module_dir).is_some() {
        script.push_str("echo 'batch_restart=audioserver'\necho 'batch_restart_started=1'\necho 'batch_restart_exit='$?\necho 'batch_restart_exit='$restart_status\n");
        script.push_str(
            render_reapply_restart_script(plan, module_dir)
                .as_deref()
                .unwrap_or_default(),
        );
    } else {
        script.push_str("echo 'batch_restart=not-required'\n");
    }
    script
}
