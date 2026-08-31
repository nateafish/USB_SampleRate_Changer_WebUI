use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use crate::android::{bluetooth_a2dp_connected_in_dump, A2dpState};
use crate::catalog::{
    BLUETOOTH_HAL_OPTIONS, JITTER_BASE_FEATURES, JITTER_FEATURES, RESAMPLER_PRESETS,
    RESAMPLER_PRESET_GROUPS,
};
use crate::cli::{
    normalize_sample_rate, parse, parse_extra_action, parse_settings_args, ControllerCommand,
};
use crate::domain::{Action, ExtraAction, ReapplyAction, Settings, StoredSettings};
use crate::operation::{
    acquire_operation_lock_at, classify_operation_result, classify_operation_state,
    classify_upstream, execute_after_state_preflight, execute_phased_mutation_with,
    execute_reapply_steps_with, persistence_allowed, render_operation_contract, script_summary,
    write_test_audit, OperationKind, OperationResult, OperationState, ProgressState,
    ReapplyProgress, ReapplyTimeouts, RestartProgress, StepProgress,
};
use crate::paths::{LOG_ROOT, STATE_ROOT};
use crate::process::{execute_output, execute_stdin, execute_stdin_with_lease, ExecutionResult};
use crate::protocol::render_schema_json_for_test;
use crate::reapply::build_reapply_plan;
use crate::scripts::{
    extra_command_summary, policy_command_summary, reapply_command_summaries,
    render_extra_restart_script, render_extra_script, render_extra_status_script,
    render_policy_script, render_reapply_script, render_reapply_verifications, shell_quote,
    upstream_args, validate_settings,
};
use crate::state::{
    parse_stored_settings, render_stored_settings, update_stored_settings_for_extra, StateDelta,
    StateHealth, StateStore,
};
use crate::{operation_not_started_fields, operation_preflight};

fn module_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn state_fixture(name: &str) -> PathBuf {
    module_fixture()
        .join("backend")
        .join("target")
        .join(format!(".phase2-{name}-{}", std::process::id()))
}

fn replace_state_value(content: &str, key: &str, value: &str) -> String {
    content
        .lines()
        .map(|line| {
            if line.starts_with(&format!("{key}=")) {
                format!("{key}={value}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn remove_state_key(content: &str, key: &str) -> String {
    content
        .lines()
        .filter(|line| !line.starts_with(&format!("{key}=")))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn rejected_state_writer(_: &Path, _: &[u8], _: u32) -> Result<(), String> {
    Err("injected atomic rewrite failure".to_string())
}

fn extra(args: &[&str]) -> ExtraAction {
    parse_extra_action(
        &args
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap()
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
fn mutation_preflight_errors_have_machine_readable_not_started_fields() {
    let mutation = operation_preflight(&[
        "usbsrctl".to_string(),
        "extra".to_string(),
        "resampler".to_string(),
        "invalid".to_string(),
    ])
    .unwrap();
    assert_eq!(
        operation_not_started_fields(&mutation),
        concat!(
            "controller_action=extra-resampler\n",
            "operation_kind=mutation\n",
            "operation_result=not_started\n",
            "operation_applied=0\n",
            "operation_state=not_started\n"
        )
    );
    let query = operation_preflight(&[
        "usbsrctl".to_string(),
        "extra".to_string(),
        "resampler".to_string(),
        "status".to_string(),
        "unexpected".to_string(),
    ])
    .unwrap();
    assert_eq!(
        operation_not_started_fields(&query),
        concat!(
            "controller_action=extra-resampler\n",
            "operation_kind=query\n",
            "operation_result=not_started\n",
            "operation_applied=0\n"
        )
    );
    assert!(operation_preflight(&["usbsrctl".to_string(), "schema".to_string()]).is_none());
    let settings = operation_preflight(&[
        "usbsrctl".to_string(),
        "settings".to_string(),
        "auto-reapply".to_string(),
        "invalid".to_string(),
    ])
    .unwrap();
    assert_eq!(
        operation_not_started_fields(&settings),
        concat!(
            "controller_action=settings-auto-reapply\n",
            "operation_kind=mutation\n",
            "operation_result=not_started\n",
            "operation_applied=0\n",
            "operation_state=not_started\n"
        )
    );
}

#[test]
fn query_and_mutation_results_have_distinct_contracts() {
    for action in [
        extra(&["bluetooth-hal", "status"]),
        extra(&["resampler", "status"]),
        extra(&["usb-period", "status"]),
        extra(&["jitter", "status"]),
        extra(&["diagnose", "audio"]),
    ] {
        assert!(action.is_query());
    }
    assert!(!extra(&["usb-period", "2250"]).is_query());

    let query =
        render_operation_contract(OperationKind::Query, OperationResult::Success, false, None);
    assert_eq!(
        query,
        "operation_kind=query\noperation_result=success\noperation_applied=0\n"
    );
    assert!(!query.contains("operation_state="));

    let mutation = render_operation_contract(
        OperationKind::Mutation,
        OperationResult::Failed,
        true,
        Some(OperationState::Applied),
    );
    assert!(mutation.contains("operation_applied=1\n"));
    assert!(mutation.contains("operation_state=applied\n"));
    assert_eq!(
        classify_operation_result(73, false, true),
        OperationResult::Failed
    );
}

#[test]
fn successful_diagnostic_query_can_persist_preferences() {
    let action = extra(&["diagnose", "alsa", "all"]);
    assert!(action.is_query());
    assert!(persistence_allowed(OperationKind::Query, true, None));
    let path = state_fixture("diagnostic-query-persistence");
    let store = StateStore::at(path.clone());
    store.update(StateDelta::Extra(action)).unwrap();
    let snapshot = store.load();
    assert_eq!(snapshot.settings.diagnostic, "alsa");
    assert!(snapshot.settings.diagnostic_all);
    let _ = fs::remove_file(path);
}

#[test]
fn query_audit_does_not_replace_last_mutation_status() {
    let root = state_fixture("query-audit");
    fs::create_dir_all(&root).unwrap();
    let status = root.join("last.status");
    fs::write(&status, "sentinel=mutation\n").unwrap();
    write_test_audit(&root, OperationKind::Query).unwrap();
    assert_eq!(fs::read_to_string(&status).unwrap(), "sentinel=mutation\n");
    let query_log = fs::read_to_string(root.join("last.log")).unwrap();
    assert!(query_log.contains("operation_kind=query\n"));
    assert!(query_log.contains("operation_result=success\n"));
    assert!(!query_log.contains("operation_state="));

    write_test_audit(&root, OperationKind::Mutation).unwrap();
    let mutation_status = fs::read_to_string(&status).unwrap();
    assert!(mutation_status.contains("last_operation_kind=mutation\n"));
    assert!(mutation_status.contains("last_operation_result=success\n"));
    assert!(mutation_status.contains("last_operation_state=applied\n"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn state_store_atomically_migrates_v1_and_v2_during_load() {
    let path = state_fixture("migration");
    for version in [1, 2] {
        fs::write(
            &path,
            format!(
                "version={version}\npolicy=usb\nsample_rate=96000\nbit_depth=24\nresampler_configured=1\nresampler_preset=default\n"
            ),
        )
        .unwrap();
        let snapshot = StateStore::at(path.clone()).load();
        assert_eq!(snapshot.migrated_from, Some(version));
        assert_eq!(snapshot.health, StateHealth::Healthy);
        assert_eq!(snapshot.settings.policy.policy, "usb");
        assert_eq!(snapshot.settings.resampler_preset, "179-408-99");

        let persisted = fs::read_to_string(&path).unwrap();
        assert!(persisted.starts_with("version=3\n"));
        assert!(persisted.contains("resampler_preset=179-408-99\n"));
        assert!(!persisted.contains("resampler_preset=default\n"));
    }
    let _ = fs::remove_file(path);
}

#[test]
fn state_store_reports_corruption_as_degraded_instead_of_defaulting_silently() {
    let path = state_fixture("corrupt");
    let invalid = replace_state_value(
        &render_stored_settings(&StoredSettings::default()),
        "sample_rate",
        "not-a-rate",
    );
    fs::write(&path, invalid).unwrap();
    let snapshot = StateStore::at(path.clone()).load();
    assert!(snapshot.health.is_degraded());
    assert!(snapshot
        .health
        .reason()
        .is_some_and(|reason| reason.contains("sample rate")));
    assert_eq!(snapshot.settings, StoredSettings::default());
    assert!(StateStore::at(path.clone())
        .update(StateDelta::SetAutoReapply(true))
        .is_err());
    StateStore::at(path.clone())
        .update(StateDelta::PolicyReset)
        .unwrap();
    assert!(fs::read_to_string(&path)
        .unwrap()
        .starts_with("version=3\n"));
    let _ = fs::remove_file(path);
}

#[test]
fn degraded_preflight_blocks_mutation_but_allows_query_and_policy_recovery() {
    let path = state_fixture("degraded-preflight");
    fs::write(&path, "version=3\nsample_rate=broken\n").unwrap();
    let last_status = path.with_extension("last.status");
    fs::write(&last_status, "sentinel=mutation\n").unwrap();
    let store = StateStore::at(path.clone());
    let mut executor_started = false;
    let blocked = execute_after_state_preflight(&store, false, || {
        executor_started = true;
        Ok(())
    });
    assert!(blocked.is_err());
    assert!(!executor_started);
    assert_eq!(
        fs::read_to_string(&last_status).unwrap(),
        "sentinel=mutation\n"
    );

    let mut query_started = false;
    execute_after_state_preflight(&store, true, || {
        query_started = true;
        Ok(())
    })
    .unwrap();
    assert!(query_started);
    assert!(store
        .update(StateDelta::Extra(extra(&["diagnose", "alsa", "all"])))
        .is_err());

    let mut reset_started = false;
    execute_after_state_preflight(&store, true, || {
        reset_started = true;
        Ok(())
    })
    .unwrap();
    assert!(reset_started);
    store.update(StateDelta::PolicyReset).unwrap();
    assert_eq!(store.load().health, StateHealth::Healthy);
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(last_status);
}

#[test]
fn renders_machine_schema_with_stable_contract_fields() {
    let schema = render_schema_json_for_test(&module_fixture());
    assert!(schema.starts_with('{') && schema.ends_with('}'));
    for field in [
        "\"schema_version\":1",
        "\"api_version\":1",
        "\"controller_version\":\"1.1.0\"",
        "\"limits\":",
        "\"policy\":{",
        "\"sample_rates\":[",
        "\"bit_depths\":[",
        "\"extras\":{",
        "\"jitter\":{",
        "\"label_key\":\"policy.option.auto.label\"",
        "\"description_key\":\"policy.option.auto.description\"",
        "\"description_key\":\"tools.bluetooth_hal.description\"",
        "\"description_key\":\"jitter.selinux.description\"",
    ] {
        assert!(schema.contains(field), "schema is missing {field}");
    }
    assert!(schema.contains("\"usb_period\":{\"min\":125"));
    assert!(schema.contains("\"features\":[{\"value\":\"selinux\""));
    assert!(schema.contains("\"templates\":["));
}

#[test]
fn renders_complete_extras_schema_capabilities() {
    let schema = render_schema_json_for_test(&module_fixture());
    for field in [
        "\"capabilities\":{\"json_schema\":true",
        "\"bluetooth_hal\":{\"available\":true,\"tool\":\"bluetooth-hal\"",
        "\"default\":\"offload\",\"recommended\":\"offload\"",
        "\"kind\":\"status\",\"selectable\":false",
        "\"actions\":[{\"value\":\"status\",\"kind\":\"status\",\"selectable\":false}",
        "\"operations\":{\"status\":true,\"set\":true,\"reset\":false}",
        "\"resampler\":{\"available\":true,\"tool\":\"resampler\"",
        "\"default_preset\":\"179-408-99\",\"upstream_default_preset\":\"default\",\"recommended\":\"179-408-99\"",
        "\"value\":\"custom\",\"label_key\":\"resampler.custom.label\"",
        "\"actions\":[{\"value\":\"status\",\"kind\":\"status\",\"selectable\":false},{\"value\":\"reset\",\"kind\":\"reset\",\"selectable\":false}",
        "\"bypass_options\":[{\"value\":\"none\"",
        "\"stop_band\":{\"min\":20,\"max\":242,\"step\":1,\"default\":179",
        "\"half_length\":{\"min\":8,\"max\":640,\"step\":8,\"default\":408}",
        "\"usb_period\":{\"available\":true,\"tool\":\"usb-period\"",
        "\"range\":{\"min\":125,\"max\":50000,\"step\":125,\"unit\":\"usec\"}",
        "\"jitter\":{\"available\":true,\"tool\":\"jitter\"",
        "\"high_risk\":true,\"requires_audio_restart\":false",
        "\"io_scheduler_options\":[{\"value\":\"*\"",
        "\"io_tone_options\":[{\"value\":\"light\"",
        "\"reset_features\":[\"selinux\",\"thermal\",\"doze\"",
        "\"diagnostics\":{\"available\":true,\"tool\":\"diagnose\"",
        "\"complete_output\":{\"supported\":true,\"argument\":\"all\"",
    ] {
        assert!(schema.contains(field), "extras schema is missing {field}");
    }
    assert!(!schema.contains("\"value\":\"default\""));
    assert!(!schema.contains("resampler.preset.default.label"));
}

#[test]
fn schema_catalog_matches_all_selectable_extra_options() {
    assert_eq!(
        BLUETOOTH_HAL_OPTIONS,
        &["offload", "aosp", "legacy", "sysbta"]
    );

    let grouped = RESAMPLER_PRESET_GROUPS
        .iter()
        .flat_map(|(_, options)| options.iter().copied())
        .filter(|value| *value != "custom")
        .collect::<std::collections::BTreeSet<_>>();
    let presets = RESAMPLER_PRESETS
        .iter()
        .map(|(value, _)| *value)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(grouped, presets);
    assert!(!presets.contains("default"));
    assert_eq!(
        RESAMPLER_PRESET_GROUPS
            .iter()
            .flat_map(|(_, options)| options.iter())
            .filter(|value| **value == "custom")
            .count(),
        1
    );
    assert!(JITTER_BASE_FEATURES
        .iter()
        .all(|feature| JITTER_FEATURES.contains(feature)));
}

#[test]
fn hidden_resampler_script_default_remains_cli_compatible() {
    let action = extra(&["resampler", "default"]);
    assert_eq!(
        action,
        ExtraAction::ResamplerPreset {
            preset: "179-408-99".to_string()
        }
    );
    assert_eq!(action.script_args(), ["--cheat", "179", "408", "99"]);
}

#[test]
fn schema_cli_keeps_text_mode_and_adds_json_mode() {
    let text = parse(&["usbsrctl".to_string(), "schema".to_string()]).unwrap();
    assert_eq!(text, ControllerCommand::Schema { json: false });
    let json = parse(&[
        "usbsrctl".to_string(),
        "schema".to_string(),
        "--json".to_string(),
    ])
    .unwrap();
    assert_eq!(json, ControllerCommand::Schema { json: true });
    assert!(parse(&[
        "usbsrctl".to_string(),
        "schema".to_string(),
        "--yaml".to_string(),
    ])
    .is_err());
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
    let script = render_policy_script(&settings, &module_fixture(), Action::Apply);
    assert!(script.contains("readlink /proc/self/ns/mnt"));
    assert!(script.contains("/core/USB_SampleRate_Changer.sh"));
    assert!(script.contains("--offload-direct"));
    assert!(script.contains("--drc"));
    assert!(script.contains("--force-usbv2"));
    assert!(script.contains("--force-bluetooth-qti"));
    assert!(script.contains("'96000' '24'"));
    assert!(script.contains("reload-audio-servers.sh"));
}

#[test]
fn script_summary_is_compact_and_does_not_embed_script() {
    let script = "#!/system/bin/sh\necho secret-command\n";
    let summary = script_summary(script);
    assert!(summary.starts_with("bytes=37;lines=2;hash="));
    assert!(!summary.contains("secret-command"));
    assert!(!summary.contains("#!/system/bin/sh"));
}

#[test]
fn controlled_executor_reads_stdin_and_caps_each_output_stream() {
    let mut shell = Command::new("/bin/sh");
    shell.arg("-s");
    let result = execute_stdin(
        &mut shell,
        "printf '0123456789'; printf 'abcdefghij' >&2\n",
        Duration::from_secs(2),
        6,
    )
    .unwrap();
    assert!(result.output.status.success());
    assert_eq!(result.output.stdout, b"012345");
    assert_eq!(result.output.stderr, b"abcdef");
    assert!(result.stdout_truncated);
    assert!(result.stderr_truncated);
    assert!(!result.timed_out);
}

#[test]
fn controlled_executor_times_out_without_terminating_the_command() {
    let marker = state_fixture("timeout-command-continues");
    let _ = fs::remove_file(&marker);
    let mut shell = Command::new("/bin/sh");
    shell.arg("-s");
    let started = Instant::now();
    let script = format!(
        "sleep 0.15\nprintf done > {}\n",
        shell_quote(&marker.to_string_lossy())
    );
    let result = execute_stdin(&mut shell, &script, Duration::from_millis(50), 1024).unwrap();
    assert!(result.timed_out);
    assert!(started.elapsed() < Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(fs::read_to_string(&marker).unwrap(), "done");
    let _ = fs::remove_file(marker);
}

#[test]
fn timed_out_mutation_keeps_flock_until_the_background_child_exits() {
    let lock_path = state_fixture("timeout-lock-keeper");
    let _ = fs::remove_file(&lock_path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let lock = acquire_operation_lock_at(&lock_path).unwrap();
    let lease = lock.lease().unwrap();
    let mut shell = Command::new("/bin/sh");
    shell.arg("-s");
    let result = execute_stdin_with_lease(
        &mut shell,
        "sleep 0.2\n",
        Duration::from_millis(30),
        1024,
        lease,
    )
    .unwrap();
    assert!(result.timed_out);
    drop(lock);
    assert!(acquire_operation_lock_at(&lock_path).is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert!(acquire_operation_lock_at(&lock_path).is_ok());
    let _ = fs::remove_file(lock_path);
}

const LOCK_KEEPER_HELPER_PATH: &str = "USB_SR_TEST_LOCK_KEEPER_PATH";
const LOCK_KEEPER_HELPER_MARKER: &str = "USB_SR_TEST_LOCK_KEEPER_MARKER";

#[test]
fn lock_keeper_survives_controller_process_exit() {
    let lock_path = state_fixture("process-exit-lock-keeper");
    let marker = state_fixture("process-exit-lock-marker");
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&marker);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::lock_keeper_process_exit_helper",
            "--nocapture",
        ])
        .env(LOCK_KEEPER_HELPER_PATH, &lock_path)
        .env(LOCK_KEEPER_HELPER_MARKER, &marker)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(acquire_operation_lock_at(&lock_path).is_err());

    let deadline = Instant::now() + Duration::from_secs(2);
    while !marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.is_file());
    while acquire_operation_lock_at(&lock_path).is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(acquire_operation_lock_at(&lock_path).is_ok());
    let _ = fs::remove_file(lock_path);
    let _ = fs::remove_file(marker);
}

#[test]
fn lock_keeper_process_exit_helper() {
    let (Ok(lock_path), Ok(marker)) = (
        std::env::var(LOCK_KEEPER_HELPER_PATH),
        std::env::var(LOCK_KEEPER_HELPER_MARKER),
    ) else {
        return;
    };
    let lock = acquire_operation_lock_at(Path::new(&lock_path)).unwrap();
    let lease = lock.lease().unwrap();
    let mut shell = Command::new("/bin/sh");
    shell.arg("-s");
    let script = format!(
        concat!(
            "for descriptor_path in /dev/fd/*; do\n",
            "  descriptor=${{descriptor_path##*/}}\n",
            "  case $descriptor in 0|1|2) ;; *) eval \"exec $descriptor>&-\" ;; esac\n",
            "done\n",
            "sleep 0.25\n",
            "printf done > {}\n"
        ),
        shell_quote(&marker)
    );
    let result =
        execute_stdin_with_lease(&mut shell, &script, Duration::from_millis(30), 1024, lease)
            .unwrap();
    assert!(result.timed_out);
    drop(lock);
    std::process::exit(0);
}

#[test]
fn controlled_query_executor_times_out_and_caps_output() {
    let mut capped = Command::new("/bin/sh");
    capped.args(["-c", "printf '0123456789'"]);
    let result = execute_output(&mut capped, Duration::from_secs(2), 5).unwrap();
    assert_eq!(result.output.stdout, b"01234");
    assert!(result.stdout_truncated);
    assert!(!result.timed_out);

    let mut timed = Command::new("/bin/sh");
    timed.args(["-c", "sleep 5 & wait"]);
    let started = Instant::now();
    let result = execute_output(&mut timed, Duration::from_millis(50), 1024).unwrap();
    assert!(result.timed_out);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn upstream_results_distinguish_success_failure_signal_and_timeout() {
    assert_eq!(classify_upstream(false, Some(0)), ("ok", 0));
    assert_eq!(classify_upstream(false, Some(17)), ("failed", 17));
    assert_eq!(classify_upstream(false, None), ("failed", 1));
    assert_eq!(classify_upstream(true, Some(143)), ("timeout", 124));
    assert_eq!(
        classify_operation_result(124, true, false),
        OperationResult::TimedOut
    );
    assert_eq!(
        classify_operation_result(71, false, false),
        OperationResult::NotStarted
    );
}

fn output_with(stdout: &str, stderr: &str, code: i32) -> std::process::Output {
    std::process::Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[test]
fn operation_state_distinguishes_not_started_and_possible_single_mutation() {
    let guard_failed = output_with("mount namespace mismatch\n", "", 71);
    assert_eq!(
        classify_operation_state(&guard_failed, false, 71),
        OperationState::NotStarted
    );

    let upstream_failed = output_with("", "controller_upstream_started=1\n", 1);
    assert_eq!(
        classify_operation_state(&upstream_failed, false, 1),
        OperationState::PossiblyApplied
    );
    assert_eq!(
        classify_operation_state(&upstream_failed, true, 124),
        OperationState::PossiblyApplied
    );
    assert_eq!(
        classify_operation_state(&output_with("", "", 0), false, 0),
        OperationState::Applied
    );
}

#[test]
fn operation_state_distinguishes_partial_reapply_batches() {
    let progress = ReapplyProgress {
        total_timeout_seconds: 60,
        total_timed_out: false,
        steps: vec![
            StepProgress {
                number: 1,
                label: "policy".into(),
                timeout_seconds: 20,
                state: ProgressState::Succeeded,
            },
            StepProgress {
                number: 2,
                label: "resampler".into(),
                timeout_seconds: 15,
                state: ProgressState::Failed(17),
            },
        ],
        restart: Some(RestartProgress {
            timeout_seconds: 20,
            state: ProgressState::Succeeded,
        }),
        verifications: Vec::new(),
    };
    assert_eq!(progress.operation_state(), OperationState::PartiallyApplied);
}

#[test]
fn extras_use_the_same_namespace_guard_as_policy() {
    let action = extra(&["usb-period", "status"]);
    let script = render_extra_script(
        &module_fixture()
            .join("core")
            .join("extras")
            .join(action.script()),
        &action,
    );
    assert!(script.contains("readlink /proc/self/ns/mnt"));
    assert!(script.contains("mount namespace mismatch"));
    assert!(script.contains("exec '/system/bin/sh'"));
    assert!(!script.contains("USB_SR_DEFER_AUDIO_RESTART"));
    assert!(!script.contains("USB_SR_RESTART_INTERFACE"));
}

#[test]
fn mutating_extra_runs_restart_and_post_status_query() {
    let action = extra(&["usb-period", "2250"]);
    let path = module_fixture()
        .join("core")
        .join("extras")
        .join(action.script());
    let business = render_extra_script(&path, &action);
    let restart = render_extra_restart_script(&path, &action).unwrap();
    let status = render_extra_status_script(&path, &action).unwrap();
    assert!(!business.contains("reload-audio-servers.sh"));
    assert!(!business.contains("--status"));
    assert!(restart.contains("reload-audio-servers.sh"));
    assert!(status.contains("--status"));
}

#[test]
fn phased_extra_distinguishes_business_restart_and_verification_results() {
    let mut calls = Vec::new();
    let restart_failed = execute_phased_mutation_with(
        "business",
        "restart",
        Some("verification"),
        |script, timeout| {
            calls.push((script.to_string(), timeout));
            Ok(fake_execution(
                if script == "restart" { 9 } else { 0 },
                true,
                false,
            ))
        },
    );
    assert_eq!(calls[0], ("business".to_string(), Duration::from_secs(15)));
    assert_eq!(calls[1], ("restart".to_string(), Duration::from_secs(20)));
    assert_eq!(calls.len(), 2);
    assert_eq!(
        restart_failed.progress.operation_state(),
        OperationState::PartiallyApplied
    );
    assert!(restart_failed.progress.persistence_ready());
    assert_eq!(
        restart_failed.progress.restart,
        Some(ProgressState::Failed(9))
    );
    assert_eq!(
        restart_failed.progress.verification,
        Some(ProgressState::SkippedPriorFailure)
    );

    calls.clear();
    let verification_failed = execute_phased_mutation_with(
        "business",
        "restart",
        Some("verification"),
        |script, timeout| {
            calls.push((script.to_string(), timeout));
            Ok(fake_execution(
                if script == "verification" { 7 } else { 0 },
                true,
                false,
            ))
        },
    );
    assert_eq!(
        calls[2],
        ("verification".to_string(), Duration::from_secs(5))
    );
    assert_eq!(
        verification_failed.progress.operation_state(),
        OperationState::PartiallyApplied
    );
    assert_eq!(
        verification_failed.progress.verification,
        Some(ProgressState::Failed(7))
    );
}

#[test]
fn reapply_builds_deduplicated_extra_verifications_after_restart() {
    let plan = vec![
        ReapplyAction::Policy(Settings::default()),
        ReapplyAction::Extra(extra(&["resampler", "179-408-99"])),
        ReapplyAction::Extra(extra(&["usb-period", "2250"])),
        ReapplyAction::Extra(extra(&["jitter", "enable", "effect"])),
    ];
    let verifications = render_reapply_verifications(&plan, &module_fixture());
    assert_eq!(verifications.len(), 3);
    assert_eq!(
        verifications
            .iter()
            .map(|step| step.label.as_str())
            .collect::<Vec<_>>(),
        vec!["resampler", "usb-period", "jitter"]
    );
    assert!(verifications
        .iter()
        .all(|step| step.script.contains("--status")));
    assert!(verifications
        .iter()
        .all(|step| !step.script.contains("USB_SampleRate_Changer.sh")));
}

#[test]
fn status_extras_do_not_claim_persisted_state_changes() {
    for action in [
        extra(&["bluetooth-hal", "status"]),
        extra(&["resampler", "status"]),
        extra(&["usb-period", "status"]),
        extra(&["jitter", "status"]),
    ] {
        assert!(!action.persists_state());
    }
    assert!(extra(&["usb-period", "2250"]).persists_state());
    assert!(extra(&["diagnose", "audio"]).persists_state());
}

#[test]
fn command_summaries_include_business_arguments_and_reapply_order() {
    let settings = Settings {
        policy: "offload-direct".to_string(),
        sample_rate: 96_000,
        bit_depth: "24".to_string(),
        ..Settings::default()
    };
    let policy = policy_command_summary(&settings, &module_fixture(), Action::Apply);
    assert!(policy.contains("USB_SampleRate_Changer.sh"));
    assert!(policy.contains("--offload-direct") && policy.contains("'96000' '24'"));

    let extra_action = extra(&["usb-period", "2250"]);
    let extra = extra_command_summary(
        &module_fixture()
            .join("core")
            .join("extras")
            .join(extra_action.script()),
        &extra_action,
    );
    assert!(extra.contains("change-usb-period.sh") && extra.ends_with("'2250'"));

    let commands = reapply_command_summaries(
        &[
            ReapplyAction::Policy(settings),
            ReapplyAction::Extra(extra_action),
        ],
        &module_fixture(),
    );
    assert_eq!(commands.len(), 4);
    assert!(commands[0].contains("USB_SampleRate_Changer.sh"));
    assert!(commands[1].contains("change-usb-period.sh"));
    assert!(commands[2].contains("reload-audio-servers.sh"));
    assert!(commands[3].contains("change-usb-period.sh"));
    assert!(commands[3].contains("--status"));
}

#[test]
fn logs_are_separated_from_persistent_state() {
    assert_eq!(STATE_ROOT, "/data/adb/usb_samplerate_changer_webui");
    assert_eq!(LOG_ROOT, "/data/local/tmp/usb_samplerate_changer_webui");
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
    assert_eq!(A2dpState::Connected.label(), "connected");
    assert_eq!(A2dpState::Disconnected.label(), "disconnected");
    assert_eq!(A2dpState::Unknown.label(), "unknown");
}

#[test]
fn extra_commands_are_strictly_whitelisted() {
    let parsed = extra(&["bluetooth-hal", "offload"]);
    assert_eq!(parsed.script(), "change-bluetooth-hal.sh");
    assert_eq!(parsed.script_args(), ["offload"]);

    let injection = ["bluetooth-hal", "offload;id"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(parse_extra_action(&injection).is_err());
}

#[test]
fn validates_usb_period_and_jitter_parameters() {
    assert!(matches!(
        extra(&["usb-period", "2250"]),
        ExtraAction::UsbPeriodSet { period: 2_250 }
    ));
    let invalid = ["usb-period", "2251"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(parse_extra_action(&invalid).is_err());

    assert_eq!(
        extra(&["jitter", "enable", "io", "*", "boost"]).script_args(),
        ["--io", "*", "boost", "--status"]
    );
    assert_eq!(
        extra(&["resampler", "custom", "96", "cheat", "194", "520", "98"]).script_args(),
        ["--bypass-hires", "--cheat", "194", "520", "98"]
    );
    assert_eq!(
        extra(&["jitter", "enable", "wifi", "no-restart"]).script_args(),
        ["--wifi-no-restart", "--status"]
    );
}

#[test]
fn a2dp_post_check_follows_audio_restart_except_for_jitter() {
    assert!(extra(&["bluetooth-hal", "offload"]).requires_a2dp_post_check());
    assert!(extra(&["resampler", "reset"]).requires_a2dp_post_check());
    assert!(extra(&["usb-period", "reset"]).requires_a2dp_post_check());
    assert!(!extra(&["jitter", "enable", "effect"]).requires_a2dp_post_check());
    assert!(!extra(&["diagnose", "audio"]).requires_a2dp_post_check());

    let jitter_reapply = ReapplyAction::Extra(extra(&["jitter", "enable", "effect"]));
    assert!(jitter_reapply.requires_audio_restart());
    assert!(!jitter_reapply.requires_a2dp_post_check());
    assert!(ReapplyAction::Policy(Settings::default()).requires_a2dp_post_check());
}

#[test]
fn stored_settings_round_trip_all_persisted_values() {
    let mut settings = StoredSettings {
        policy_configured: true,
        bluetooth_hal: "aosp".to_string(),
        bluetooth_hal_configured: true,
        resampler_preset: "custom".to_string(),
        resampler_configured: true,
        resampler_bypass: "96".to_string(),
        resampler_stop_band: 194,
        resampler_half_length: 520,
        resampler_percent: 98,
        usb_period: 1_000,
        usb_period_configured: true,
        io_scheduler: "bfq".to_string(),
        io_tone: "boost".to_string(),
        auto_reapply: true,
        ..StoredSettings::default()
    };
    settings.policy.policy = "usb".to_string();
    settings.jitter_values.insert("io".to_string(), true);
    settings.jitter_configured.insert("io".to_string(), true);

    assert_eq!(
        parse_stored_settings(&render_stored_settings(&settings)),
        settings
    );
}

#[test]
fn complete_v3_round_trip_is_healthy_and_accepts_empty_template() {
    let path = state_fixture("complete-v3");
    let settings = StoredSettings::default();
    let rendered = render_stored_settings(&settings);
    assert!(rendered.contains("test_template=\n"));
    fs::write(&path, &rendered).unwrap();

    let snapshot = StateStore::at(path.clone()).load();
    assert_eq!(snapshot.health, StateHealth::Healthy);
    assert_eq!(snapshot.settings, settings);
    assert_eq!(fs::read_to_string(&path).unwrap(), rendered);
    let _ = fs::remove_file(path);
}

#[test]
fn strict_v3_rejects_every_missing_non_version_key() {
    let path = state_fixture("missing-v3-key");
    let complete = render_stored_settings(&StoredSettings::default());
    for line in complete
        .lines()
        .filter(|line| !line.starts_with("version="))
    {
        let key = line.split_once('=').unwrap().0;
        fs::write(&path, remove_state_key(&complete, key)).unwrap();
        let snapshot = StateStore::at(path.clone()).load();
        assert!(snapshot.health.is_degraded(), "missing {key} was accepted");
        assert_eq!(
            snapshot.health.reason(),
            Some(format!("missing state key: {key}").as_str()),
            "wrong error for missing {key}"
        );
    }
    let _ = fs::remove_file(path);
}

#[test]
fn strict_v3_rejects_duplicate_unknown_and_truncated_lines() {
    let path = state_fixture("v3-structure");
    let complete = render_stored_settings(&StoredSettings::default());

    fs::write(&path, format!("{complete}auto_reapply=1\n")).unwrap();
    let duplicate = StateStore::at(path.clone()).load();
    assert_eq!(
        duplicate.health.reason(),
        Some("duplicate state key: auto_reapply")
    );

    fs::write(&path, format!("{complete}future_option=1\n")).unwrap();
    let unknown = StateStore::at(path.clone()).load();
    assert!(unknown.health.reason().is_some_and(
        |reason| reason.contains("unknown state key") && reason.contains("future_option")
    ));

    fs::write(&path, format!("{complete}truncated-line")).unwrap();
    let truncated = StateStore::at(path.clone()).load();
    assert!(truncated
        .health
        .reason()
        .is_some_and(|reason| reason.contains("malformed state line")));
    let _ = fs::remove_file(path);
}

#[test]
fn unversioned_mixed_state_salvages_only_valid_values_and_rewrites_v3() {
    let path = state_fixture("unversioned-salvage");
    let legacy = concat!(
        "policy=usb\n",
        "sample_rate=96000\n",
        "bit_depth=invalid\n",
        "auto_reapply=1\n",
        "future_option=1\n",
        "malformed-line\n",
        "resampler_configured=1\n",
        "resampler_preset=custom\n",
        "resampler_bypass=96\n",
        "resampler_cheat=0\n",
        "resampler_stop_band=194\n",
        "resampler_half_length=520\n",
        "resampler_percent=101\n"
    );
    fs::write(&path, legacy).unwrap();

    let snapshot = StateStore::at(path.clone()).load();
    assert_eq!(snapshot.health, StateHealth::Healthy);
    assert_eq!(snapshot.migrated_from, None);
    assert!(snapshot
        .recovery_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("unversioned legacy state salvaged")));
    assert_eq!(snapshot.settings.policy.policy, "usb");
    assert_eq!(snapshot.settings.policy.sample_rate, 96_000);
    assert_eq!(snapshot.settings.policy.bit_depth, "32");
    assert!(snapshot.settings.auto_reapply);
    assert_eq!(snapshot.settings.resampler_preset, "179-408-99");
    assert!(!snapshot.settings.resampler_configured);

    let rewritten = fs::read_to_string(&path).unwrap();
    assert_eq!(rewritten, render_stored_settings(&snapshot.settings));
    assert!(rewritten.starts_with("version=3\n"));
    assert!(!rewritten.contains("future_option"));
    assert!(!rewritten.contains("malformed-line"));
    let _ = fs::remove_file(path);
}

#[test]
fn empty_unversioned_file_rewrites_complete_defaults_but_missing_file_does_not() {
    let empty_path = state_fixture("empty-unversioned");
    fs::write(&empty_path, "").unwrap();
    let empty = StateStore::at(empty_path.clone()).load();
    assert_eq!(empty.health, StateHealth::Healthy);
    assert_eq!(empty.settings, StoredSettings::default());
    assert_eq!(
        empty.recovery_reason.as_deref(),
        Some("empty unversioned state rebuilt from defaults")
    );
    assert_eq!(
        fs::read_to_string(&empty_path).unwrap(),
        render_stored_settings(&StoredSettings::default())
    );

    let missing_path = state_fixture("missing-state");
    let _ = fs::remove_file(&missing_path);
    let missing = StateStore::at(missing_path.clone()).load();
    assert_eq!(missing.health, StateHealth::Healthy);
    assert_eq!(missing.recovery_reason, None);
    assert!(!missing_path.exists());
    let _ = fs::remove_file(empty_path);
}

#[test]
fn unversioned_rewrite_failure_is_degraded_and_preserves_original() {
    let path = state_fixture("rewrite-failure");
    let original = "policy=usb\nsample_rate=96000\n";
    fs::write(&path, original).unwrap();
    let snapshot = StateStore::at_with_writer(path.clone(), rejected_state_writer).load();
    assert!(snapshot.health.is_degraded());
    assert!(snapshot
        .health
        .reason()
        .is_some_and(|reason| reason.contains("state rewrite failed")
            && reason.contains("injected atomic rewrite failure")));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let _ = fs::remove_file(path);
}

#[test]
fn persisted_sample_rate_requires_numeric_custom_bounds() {
    let path = state_fixture("sample-rate-bounds");
    for (value, expected) in [
        ("44100", 44_100),
        ("768000", 768_000),
        ("44099", 44_100),
        ("768001", 44_100),
        ("custom", 44_100),
        ("96k", 44_100),
    ] {
        fs::write(&path, format!("sample_rate={value}\n")).unwrap();
        let snapshot = StateStore::at(path.clone()).load();
        assert_eq!(snapshot.health, StateHealth::Healthy);
        assert_eq!(snapshot.settings.policy.sample_rate, expected, "{value}");
    }

    let complete = render_stored_settings(&StoredSettings::default());
    for value in ["44100", "768000"] {
        fs::write(&path, replace_state_value(&complete, "sample_rate", value)).unwrap();
        assert_eq!(
            StateStore::at(path.clone()).load().health,
            StateHealth::Healthy,
            "strict v3 rejected {value}"
        );
    }
    for value in ["44099", "768001", "custom", "96k"] {
        fs::write(&path, replace_state_value(&complete, "sample_rate", value)).unwrap();
        assert!(
            StateStore::at(path.clone()).load().health.is_degraded(),
            "strict v3 accepted {value}"
        );
    }
    let _ = fs::remove_file(path);
}

#[test]
fn strict_v3_validates_custom_resampler_as_one_group() {
    let path = state_fixture("custom-resampler");
    let mut custom = StoredSettings {
        resampler_preset: "custom".to_string(),
        resampler_configured: true,
        resampler_bypass: "none".to_string(),
        resampler_cheat: false,
        resampler_stop_band: 20,
        resampler_half_length: 8,
        resampler_percent: 100,
        ..StoredSettings::default()
    };
    fs::write(&path, render_stored_settings(&custom)).unwrap();
    assert_eq!(
        StateStore::at(path.clone()).load().health,
        StateHealth::Healthy
    );
    let bypass_48 = replace_state_value(&render_stored_settings(&custom), "resampler_bypass", "48");
    fs::write(&path, bypass_48).unwrap();
    assert_eq!(
        StateStore::at(path.clone()).load().health,
        StateHealth::Healthy
    );

    custom.resampler_percent = 101;
    fs::write(&path, render_stored_settings(&custom)).unwrap();
    assert!(StateStore::at(path.clone()).load().health.is_degraded());
    custom.resampler_cheat = true;
    custom.resampler_percent = 200;
    custom.resampler_stop_band = 242;
    custom.resampler_half_length = 640;
    custom.resampler_bypass = "96".to_string();
    fs::write(&path, render_stored_settings(&custom)).unwrap();
    assert_eq!(
        StateStore::at(path.clone()).load().health,
        StateHealth::Healthy
    );

    for (key, value) in [
        ("resampler_percent", "201"),
        ("resampler_half_length", "7"),
        ("resampler_half_length", "648"),
        ("resampler_half_length", "10"),
        ("resampler_stop_band", "19"),
        ("resampler_stop_band", "243"),
        ("resampler_bypass", "48k"),
    ] {
        let invalid = replace_state_value(&render_stored_settings(&custom), key, value);
        fs::write(&path, invalid).unwrap();
        assert!(
            StateStore::at(path.clone()).load().health.is_degraded(),
            "strict v3 accepted {key}={value}"
        );
    }

    let missing_group = remove_state_key(&render_stored_settings(&custom), "resampler_half_length");
    fs::write(&path, missing_group).unwrap();
    assert_eq!(
        StateStore::at(path.clone()).load().health.reason(),
        Some("missing state key: resampler_half_length")
    );
    let _ = fs::remove_file(path);
}

#[test]
fn unversioned_incomplete_custom_resampler_falls_back_without_half_custom() {
    let path = state_fixture("incomplete-custom-salvage");
    fs::write(
        &path,
        concat!(
            "resampler_configured=1\n",
            "resampler_preset=custom\n",
            "resampler_bypass=48\n",
            "resampler_cheat=0\n",
            "resampler_stop_band=194\n",
            "resampler_half_length=520\n",
            "resampler_percent=100\n"
        ),
    )
    .unwrap();
    let complete = StateStore::at(path.clone()).load();
    assert_eq!(complete.health, StateHealth::Healthy);
    assert_eq!(complete.settings.resampler_preset, "custom");
    assert!(complete.settings.resampler_configured);

    fs::write(
        &path,
        concat!(
            "resampler_configured=1\n",
            "resampler_preset=custom\n",
            "resampler_bypass=48\n",
            "resampler_cheat=1\n",
            "resampler_stop_band=194\n",
            "resampler_percent=150\n"
        ),
    )
    .unwrap();
    let snapshot = StateStore::at(path.clone()).load();
    let defaults = StoredSettings::default();
    assert_eq!(snapshot.health, StateHealth::Healthy);
    assert_eq!(
        snapshot.settings.resampler_preset,
        defaults.resampler_preset
    );
    assert_eq!(
        snapshot.settings.resampler_bypass,
        defaults.resampler_bypass
    );
    assert_eq!(
        snapshot.settings.resampler_half_length,
        defaults.resampler_half_length
    );
    assert!(!snapshot.settings.resampler_configured);
    let _ = fs::remove_file(path);
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
fn legacy_resampler_default_is_canonicalized_on_read_and_write() {
    let content = replace_state_value(
        &replace_state_value(
            &render_stored_settings(&StoredSettings::default()),
            "resampler_configured",
            "1",
        ),
        "resampler_preset",
        "default",
    );
    let path = state_fixture("resampler-default");
    fs::write(&path, content).unwrap();
    let store = StateStore::at(path.clone());
    let snapshot = store.load();
    assert_eq!(snapshot.health, StateHealth::Healthy);
    assert_eq!(snapshot.settings.resampler_preset, "179-408-99");
    assert!(snapshot.settings.resampler_configured);

    store
        .update(StateDelta::SetAutoReapply(true))
        .expect("canonicalized state should remain writable");
    let persisted = fs::read_to_string(path).unwrap();
    assert!(persisted.contains("resampler_preset=179-408-99\n"));
    assert!(!persisted.contains("resampler_preset=default\n"));
}

#[test]
fn extra_updates_every_tool_and_tuning_selection() {
    let mut settings = StoredSettings::default();
    let policy = settings.policy.clone();
    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["bluetooth-hal", "aosp"])
    ));
    assert!(settings.bluetooth_hal_configured);
    assert_eq!(settings.bluetooth_hal, "aosp");

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["resampler", "custom", "96", "cheat", "194", "520", "98"])
    ));
    assert!(settings.resampler_configured);
    assert_eq!(settings.resampler_preset, "custom");
    assert_eq!(settings.resampler_bypass, "96");
    assert_eq!(settings.resampler_stop_band, 194);
    assert_eq!(settings.resampler_half_length, 520);
    assert_eq!(settings.resampler_percent, 98);
    assert_eq!(settings.policy, policy);

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["resampler", "default"])
    ));
    assert_eq!(settings.resampler_preset, "179-408-99");

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["usb-period", "1000"])
    ));
    assert!(settings.usb_period_configured);
    assert_eq!(settings.usb_period, 1_000);

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["diagnose", "alsa", "all"])
    ));
    assert_eq!(settings.diagnostic, "alsa");
    assert!(settings.diagnostic_all);

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["jitter", "enable", "io", "bfq", "boost"])
    ));
    assert_eq!(settings.jitter_values.get("io"), Some(&true));
    assert_eq!(settings.jitter_configured.get("io"), Some(&true));
    assert_eq!(settings.io_scheduler, "bfq");
    assert_eq!(settings.io_tone, "boost");

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["jitter", "enable", "wifi", "no-restart"])
    ));
    assert_eq!(settings.jitter_values.get("wifi"), Some(&true));
    assert!(settings.wifi_no_restart);

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["jitter", "disable", "battery"])
    ));
    assert_eq!(settings.jitter_values.get("battery"), Some(&false));
    assert_eq!(settings.jitter_configured.get("battery"), Some(&true));

    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["resampler", "reset"])
    ));
    assert!(!settings.resampler_configured);
    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["usb-period", "reset"])
    ));
    assert!(!settings.usb_period_configured);
    assert!(update_stored_settings_for_extra(
        &mut settings,
        &extra(&["jitter", "disable", "all"])
    ));
    for feature in crate::catalog::JITTER_BASE_FEATURES {
        assert_eq!(settings.jitter_values.get(*feature), Some(&false));
        assert_eq!(settings.jitter_configured.get(*feature), Some(&false));
    }
}

#[test]
fn reapply_plan_replays_only_applied_values_in_dependency_order() {
    let mut settings = StoredSettings {
        bluetooth_hal: "aosp".to_string(),
        bluetooth_hal_configured: true,
        policy_configured: true,
        resampler_preset: "custom".to_string(),
        resampler_configured: true,
        resampler_bypass: "96".to_string(),
        resampler_stop_band: 194,
        resampler_half_length: 520,
        resampler_percent: 98,
        usb_period: 1_000,
        usb_period_configured: true,
        io_scheduler: "bfq".to_string(),
        io_tone: "boost".to_string(),
        diagnostic: "alsa".to_string(),
        ..StoredSettings::default()
    };
    settings.policy.policy = "usb".to_string();
    settings.jitter_values.insert("thermal".to_string(), false);
    settings
        .jitter_configured
        .insert("thermal".to_string(), true);
    settings.jitter_values.insert("io".to_string(), true);
    settings.jitter_configured.insert("io".to_string(), true);

    assert!(build_reapply_plan(&settings).is_empty());
    settings.auto_reapply = true;
    assert_eq!(
        build_reapply_plan(&settings),
        vec![
            ReapplyAction::Extra(ExtraAction::BluetoothHal {
                action: "aosp".into()
            }),
            ReapplyAction::Policy(settings.policy.clone()),
            ReapplyAction::Extra(ExtraAction::ResamplerCustom {
                bypass: "96".into(),
                cheat: true,
                stop_band: 194,
                half_length: 520,
                percent: 98,
            }),
            ReapplyAction::Extra(ExtraAction::UsbPeriodSet { period: 1_000 }),
            ReapplyAction::Extra(ExtraAction::JitterSet {
                enabled: false,
                feature: "thermal".into(),
                scheduler: None,
                tone: None,
                wifi_no_restart: false,
            }),
            ReapplyAction::Extra(ExtraAction::JitterSet {
                enabled: true,
                feature: "io".into(),
                scheduler: Some("bfq".into()),
                tone: Some("boost".into()),
                wifi_no_restart: false,
            }),
        ]
    );
}

#[test]
fn reapply_batch_defers_and_coalesces_audio_restarts() {
    let plan = vec![
        ReapplyAction::Extra(extra(&["bluetooth-hal", "offload"])),
        ReapplyAction::Policy(Settings::default()),
        ReapplyAction::Extra(extra(&["resampler", "179-408-99"])),
        ReapplyAction::Extra(extra(&["usb-period", "2250"])),
        ReapplyAction::Extra(extra(&["jitter", "enable", "effect"])),
    ];
    let script = render_reapply_script(&plan, &module_fixture());
    assert!(!script.contains("USB_SR_DEFER_AUDIO_RESTART"));
    assert!(!script.contains("USB_SR_RESTART_INTERFACE"));
    assert!(script.contains("reload-audio-servers.sh"));
    assert!(script.contains("'--bluetooth-hal' 'offload'"));
    assert_eq!(script.matches("batch_restart=audioserver").count(), 1);
    assert!(script.contains("batch_step=1:bluetooth-hal"));
    assert!(script.contains("batch_step=5:jitter"));
    assert!(script.contains("batch_step_started=1"));
    assert!(script.contains("batch_step_exit=1:$step_status"));
    assert!(script.contains("batch_restart_started=1"));
    assert!(script.contains("batch_restart_exit='$?"));
}

#[test]
fn non_audio_reapply_batch_does_not_restart_audioserver() {
    let plan = vec![ReapplyAction::Extra(extra(&[
        "jitter", "enable", "thermal",
    ]))];
    let script = render_reapply_script(&plan, &module_fixture());
    assert!(!script.contains("USB_SR_DEFER_AUDIO_RESTART"));
    assert!(!script.contains("USB_SR_RESTART_INTERFACE"));
    assert!(!script.contains("reload-audio-servers.sh"));
    assert!(script.contains("batch_restart=not-required"));
}

fn fake_execution(code: i32, started: bool, timed_out: bool) -> ExecutionResult {
    ExecutionResult {
        output: output_with(
            "",
            if started {
                "controller_upstream_started=1\n"
            } else {
                ""
            },
            code,
        ),
        timed_out,
        stdout_truncated: false,
        stderr_truncated: false,
    }
}

#[test]
fn reapply_enforces_independent_step_timeout_and_partial_state_markers() {
    let plan = vec![
        ReapplyAction::Extra(extra(&["usb-period", "2250"])),
        ReapplyAction::Extra(extra(&["usb-period", "2250"])),
    ];
    let steps = vec![
        crate::scripts::ReapplyScriptStep {
            label: "first".into(),
            script: "one".into(),
        },
        crate::scripts::ReapplyScriptStep {
            label: "second".into(),
            script: "two".into(),
        },
    ];
    let mut calls = 0;
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        None,
        &[],
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(2),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(10),
        },
        || Duration::from_secs(0),
        |_script, timeout| {
            calls += 1;
            assert_eq!(timeout, Duration::from_secs(3));
            if calls == 1 {
                Ok(fake_execution(0, true, false))
            } else {
                Ok(fake_execution(124, true, true))
            }
        },
    );
    let stdout = String::from_utf8_lossy(&result.execution.output.stdout);
    assert!(stdout.contains("batch_step_timeout_seconds=1:3"));
    assert!(stdout.contains("batch_step_timed_out=2:1"));
    assert!(stdout.contains("batch_timeout_scope=step:2"));
    assert!(stdout.contains("batch_step_exit=1:0"));
    assert!(stdout.contains("batch_step_exit=2:124"));
    assert!(result.execution.timed_out);
    assert_eq!(result.execution.output.status.code(), Some(124));
    assert_eq!(
        result.progress.operation_state(),
        OperationState::PartiallyApplied
    );
}

#[test]
fn reapply_restart_has_an_independent_timeout() {
    let plan = vec![ReapplyAction::Extra(extra(&["usb-period", "2250"]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "usb-period".into(),
        script: "step".into(),
    }];
    let mut calls = 0;
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        Some("restart"),
        &[],
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(4),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(10),
        },
        || Duration::from_secs(0),
        |_script, timeout| {
            calls += 1;
            if calls == 1 {
                assert_eq!(timeout, Duration::from_secs(3));
                Ok(fake_execution(0, true, false))
            } else {
                assert_eq!(timeout, Duration::from_secs(4));
                Ok(fake_execution(124, true, true))
            }
        },
    );
    let stdout = String::from_utf8_lossy(&result.execution.output.stdout);
    assert!(stdout.contains("batch_restart_timeout_seconds=4"));
    assert!(stdout.contains("batch_restart_timed_out=1"));
    assert!(stdout.contains("batch_restart_exit=124"));
    assert!(stdout.contains("batch_timeout_scope=restart"));
    assert!(result.execution.timed_out);
    assert_eq!(result.execution.output.status.code(), Some(124));
    assert_eq!(
        result.progress.operation_state(),
        OperationState::PartiallyApplied
    );
}

#[test]
fn reapply_total_cap_skips_restart_after_business_step() {
    let plan = vec![ReapplyAction::Extra(extra(&["usb-period", "2250"]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "usb-period".into(),
        script: "step".into(),
    }];
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        Some("restart"),
        &[],
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(4),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(5),
        },
        {
            let mut elapsed = 0;
            move || {
                elapsed += 3;
                Duration::from_secs(elapsed)
            }
        },
        |_script, timeout| {
            assert!(timeout <= Duration::from_secs(3));
            Ok(fake_execution(0, true, false))
        },
    );
    let stdout = String::from_utf8_lossy(&result.execution.output.stdout);
    assert!(stdout.contains("batch_restart_skipped=total-timeout"));
    assert!(stdout.contains("batch_timeout_scope=total"));
    assert!(stdout.contains("batch_total_timed_out=1"));
    assert!(result.execution.timed_out);
    assert_eq!(result.execution.output.status.code(), Some(124));
}

#[test]
fn reapply_progress_ignores_spoofed_markers_and_truncated_output() {
    let plan = vec![ReapplyAction::Extra(extra(&[
        "jitter", "enable", "thermal",
    ]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "jitter".into(),
        script: "step".into(),
    }];
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        None,
        &[],
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(4),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(10),
        },
        || Duration::ZERO,
        |_script, _timeout| {
            let mut spoofed = b"batch_step_exit=1:0\nbatch_restart_exit=0\n".to_vec();
            spoofed.resize(70 * 1024, b'x');
            Ok(ExecutionResult {
                output: std::process::Output {
                    status: std::process::ExitStatus::from_raw(17 << 8),
                    stdout: spoofed,
                    stderr: Vec::new(),
                },
                timed_out: false,
                stdout_truncated: false,
                stderr_truncated: false,
            })
        },
    );
    assert!(result.execution.stdout_truncated);
    assert_eq!(
        result.progress.operation_state(),
        OperationState::PossiblyApplied
    );
    assert_eq!(result.progress.steps[0].state, ProgressState::Failed(17));
    let control = String::from_utf8_lossy(&result.execution.output.stdout);
    assert!(control.starts_with("batch_total_timeout_seconds=10\n"));
    assert!(control.contains("batch_step_exit=1:17\n"));
    assert_eq!(control.matches("batch_step_exit=1:").count(), 1);
    assert!(!control.contains("batch_restart_exit=0\n"));
}

#[test]
fn reapply_progress_models_first_timeout_and_all_start_failure() {
    let plan = vec![ReapplyAction::Extra(extra(&[
        "jitter", "enable", "thermal",
    ]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "jitter".into(),
        script: "step".into(),
    }];
    let timeouts = ReapplyTimeouts {
        policy: Duration::from_secs(20),
        extra: Duration::from_secs(3),
        restart: Duration::from_secs(4),
        verification: Duration::from_secs(2),
        total: Duration::from_secs(10),
    };
    let timed_out = execute_reapply_steps_with(
        &plan,
        &steps,
        None,
        &[],
        timeouts,
        || Duration::ZERO,
        |_script, _timeout| Ok(fake_execution(124, false, true)),
    );
    assert_eq!(
        timed_out.progress.operation_state(),
        OperationState::PossiblyApplied
    );
    assert_eq!(
        timed_out.progress.steps[0].state,
        ProgressState::TimedOut {
            total_budget: false
        }
    );

    let start_failed = execute_reapply_steps_with(
        &plan,
        &steps,
        None,
        &[],
        timeouts,
        || Duration::ZERO,
        |_script, _timeout| Err("spawn failed".to_string()),
    );
    assert_eq!(
        start_failed.progress.operation_state(),
        OperationState::NotStarted
    );
    assert_eq!(
        start_failed.progress.steps[0].state,
        ProgressState::StartFailed
    );
}

#[test]
fn reapply_progress_models_non_timeout_restart_failure() {
    let plan = vec![ReapplyAction::Extra(extra(&["usb-period", "2250"]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "usb-period".into(),
        script: "step".into(),
    }];
    let mut calls = 0;
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        Some("restart"),
        &[],
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(4),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(10),
        },
        || Duration::ZERO,
        |_script, _timeout| {
            calls += 1;
            Ok(fake_execution(if calls == 1 { 0 } else { 9 }, false, false))
        },
    );
    assert_eq!(
        result.progress.operation_state(),
        OperationState::PartiallyApplied
    );
    assert_eq!(
        result.progress.restart.unwrap().state,
        ProgressState::Failed(9)
    );
}

#[test]
fn reapply_runs_independent_verifications_and_stops_after_timeout() {
    let plan = vec![ReapplyAction::Extra(extra(&["usb-period", "2250"]))];
    let steps = vec![crate::scripts::ReapplyScriptStep {
        label: "usb-period".into(),
        script: "business".into(),
    }];
    let verifications = vec![
        crate::scripts::ReapplyScriptStep {
            label: "usb-period".into(),
            script: "verify-one".into(),
        },
        crate::scripts::ReapplyScriptStep {
            label: "resampler".into(),
            script: "verify-two".into(),
        },
    ];
    let mut calls = Vec::new();
    let result = execute_reapply_steps_with(
        &plan,
        &steps,
        Some("restart"),
        &verifications,
        ReapplyTimeouts {
            policy: Duration::from_secs(20),
            extra: Duration::from_secs(3),
            restart: Duration::from_secs(4),
            verification: Duration::from_secs(2),
            total: Duration::from_secs(20),
        },
        || Duration::ZERO,
        |script, timeout| {
            calls.push((script.to_string(), timeout));
            if script == "verify-one" {
                Ok(fake_execution(124, true, true))
            } else {
                Ok(fake_execution(0, true, false))
            }
        },
    );
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2], ("verify-one".to_string(), Duration::from_secs(2)));
    assert_eq!(
        result.progress.verifications[0].state,
        ProgressState::TimedOut {
            total_budget: false
        }
    );
    assert_eq!(
        result.progress.verifications[1].state,
        ProgressState::SkippedPriorFailure
    );
    assert_eq!(
        result.progress.operation_state(),
        OperationState::PartiallyApplied
    );
    let markers = String::from_utf8_lossy(&result.execution.output.stdout);
    assert!(markers.contains("batch_verification_exit=1:124"));
    assert!(markers.contains("batch_verification_skipped=2:prior-phase-failure-or-timeout"));
}
