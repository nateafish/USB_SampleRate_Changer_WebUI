mod aidl;
mod aidl_backend;
mod android;
mod audioserver_priority;
mod catalog;
mod cli;
mod domain;
mod dynamic_direct;
mod operation;
mod paths;
mod process;
mod protocol;
mod reapply;
mod scripts;
mod state;

use std::env;

use cli::ControllerCommand;
use domain::{Action, Settings};
use operation::{
    acquire_operation_lock, finish_internal_mutation, load_state_preflight,
    render_operation_contract, run_cleanup, run_extra, run_operation, run_reapply_batch_locked,
    OperationKind, OperationResult, OperationState,
};
use paths::{ensure_state_layout, module_dir};
use protocol::{print_generated, print_logs, print_schema, print_schema_json, print_status};
use reapply::build_reapply_plan;
use scripts::{render_policy_script, validate_settings};
use state::{bool_number, StateDelta, StateStore};

pub fn main_entry() {
    restore_default_sigpipe();
    let args: Vec<String> = env::args().collect();
    let operation = operation_preflight(&args);
    match run(&args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            if let Some(operation) = operation {
                print!("{}", operation_not_started_fields(&operation));
            }
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

fn run(args: &[String]) -> Result<i32, String> {
    // Internal entry point runs under the outer controller's lock and mount
    // namespace guard. It must not acquire that lock recursively.
    if args.get(1).map(String::as_str) == Some("_dynamic-direct") {
        let settings = cli::parse_settings_args(&args[2..])?;
        dynamic_direct::apply(&settings, &module_dir()?)?;
        return Ok(0);
    }
    match args.get(1).map(String::as_str) {
        Some("_audioserver-priority-watch") if args.len() == 2 => {
            audioserver_priority::watch_loop()?;
            return Ok(0);
        }
        Some("_audioserver-priority-start") if args.len() == 2 => {
            audioserver_priority::start_watch()?;
            return Ok(0);
        }
        Some("_audioserver-priority-stop") if args.len() == 2 => {
            audioserver_priority::stop_watch()?;
            return Ok(0);
        }
        _ => {}
    }
    match cli::parse(args)? {
        ControllerCommand::Schema { json } => {
            let module_dir = module_dir()?;
            if json {
                print_schema_json(&module_dir);
            } else {
                print_schema(&module_dir);
            }
            Ok(0)
        }
        ControllerCommand::Status => {
            print_status(&module_dir()?);
            Ok(0)
        }
        ControllerCommand::Logs => {
            print_logs()?;
            Ok(0)
        }
        ControllerCommand::Generated => {
            print_generated()?;
            Ok(0)
        }
        ControllerCommand::AidlInspect => aidl_backend::print_inspect().map(|_| 0),
        ControllerCommand::AidlInspectStdin => aidl_backend::print_inspect_stdin().map(|_| 0),
        ControllerCommand::AidlPreview {
            policy,
            rate,
            bit_depth,
        } => aidl_backend::print_preview(&policy, &rate, &bit_depth).map(|_| 0),
        ControllerCommand::AidlMatchStdin { xml } => {
            aidl_backend::print_match_stdin(&xml).map(|_| 0)
        }
        ControllerCommand::AidlRewriteValidateStdin {
            xml,
            port,
            rate,
            bit_depth,
        } => aidl_backend::print_rewrite_validate_stdin(&xml, &port, &rate, &bit_depth).map(|_| 0),
        ControllerCommand::AidlPlanStdin {
            xml,
            policy,
            rate,
            bit_depth,
            output,
        } => aidl_backend::print_plan_stdin(&xml, &policy, &rate, &bit_depth, output.as_deref())
            .map(|_| 0),
        ControllerCommand::AidlRestore => aidl_backend::restore(),
        ControllerCommand::Preview(settings) => {
            let module_dir = module_dir()?;
            validate_settings(&settings, &module_dir)?;
            print!(
                "{}",
                render_policy_script(&settings, &module_dir, Action::Apply)
            );
            Ok(0)
        }
        ControllerCommand::Apply(settings) => run_operation(settings, Action::Apply),
        ControllerCommand::Reset => run_operation(Settings::default(), Action::Reset),
        ControllerCommand::Cleanup => run_cleanup(),
        ControllerCommand::Extra(action) => run_extra(action),
        ControllerCommand::SetAutoReapply(enabled) => run_settings_command(enabled),
        ControllerCommand::SetAudioserverPriority(enabled) => {
            run_audioserver_priority_setting(enabled)
        }
        ControllerCommand::Reapply => run_reapply(),
    }
}

fn run_audioserver_priority_setting(enabled: bool) -> Result<i32, String> {
    ensure_state_layout()?;
    let (code, change) = {
        // The feature owns a separate marker and baseline, so it remains
        // possible to disable it even when settings.conf is degraded.
        let _lock = acquire_operation_lock()?;
        let change = audioserver_priority::set_enabled(enabled)?;
        let code = finish_internal_mutation(
            "settings-audioserver-priority",
            format!(
                "usbsrctl settings audioserver-priority {}",
                if enabled { "enable" } else { "disable" }
            ),
        )?;
        (code, change)
    };

    let monitor_result = if enabled {
        audioserver_priority::start_watch()
    } else {
        audioserver_priority::stop_watch()
    };
    if let Err(error) = monitor_result {
        eprintln!("WARNING: audioserver priority monitor update failed: {error}");
    }
    println!("audioserver_priority={}", bool_number(enabled));
    println!(
        "audioserver_priority_target={}",
        audioserver_priority::TARGET_NICE
    );
    println!("audioserver_priority_adjusted={}", change.adjusted);
    println!("audioserver_priority_restored={}", change.restored);
    Ok(code)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationPreflight {
    action: String,
    kind: OperationKind,
}

pub(crate) fn operation_preflight(args: &[String]) -> Option<OperationPreflight> {
    match args.get(1).map(String::as_str) {
        Some("apply" | "reset" | "cleanup" | "reapply") => Some(OperationPreflight {
            action: args[1].clone(),
            kind: OperationKind::Mutation,
        }),
        Some("aidl-restore") => Some(OperationPreflight {
            action: "aidl-restore".to_string(),
            kind: OperationKind::Mutation,
        }),
        Some("settings") => Some(OperationPreflight {
            action: match (
                args.get(2).map(String::as_str),
                args.get(3).map(String::as_str),
            ) {
                (Some("auto-reapply"), _) => "settings-auto-reapply".to_string(),
                (Some("audioserver-priority"), _) => "settings-audioserver-priority".to_string(),
                _ => "settings".to_string(),
            },
            kind: OperationKind::Mutation,
        }),
        Some("extra") => {
            let tool = args.get(2).map(String::as_str);
            let raw_action = args.get(3).map(String::as_str);
            let kind = if tool == Some("diagnose")
                || (matches!(
                    tool,
                    Some("bluetooth-hal" | "resampler" | "usb-period" | "jitter")
                ) && raw_action == Some("status"))
            {
                OperationKind::Query
            } else {
                OperationKind::Mutation
            };
            let action = match (tool, raw_action) {
                (Some("diagnose"), Some(kind)) => format!("extra-diagnose-{kind}"),
                (Some(tool), _) => format!("extra-{tool}"),
                (None, _) => "extra".to_string(),
            };
            Some(OperationPreflight { action, kind })
        }
        _ => None,
    }
}

pub(crate) fn operation_not_started_fields(operation: &OperationPreflight) -> String {
    let mut fields = format!(
        "controller_action={}\noperation_kind={}\noperation_result=not_started\noperation_applied=0\n",
        operation.action,
        operation.kind.label()
    );
    if operation.kind == OperationKind::Mutation {
        fields.push_str("operation_state=not_started\n");
    }
    fields
}

fn run_settings_command(enabled: bool) -> Result<i32, String> {
    ensure_state_layout()?;
    let _lock = acquire_operation_lock()?;
    let store = StateStore::new();
    load_state_preflight(&store, false)?;
    store.update(StateDelta::SetAutoReapply(enabled))?;
    let code = finish_internal_mutation(
        "settings-auto-reapply",
        format!(
            "usbsrctl settings auto-reapply {}",
            if enabled { "enable" } else { "disable" }
        ),
    )?;
    println!("auto_reapply={}", bool_number(enabled));
    Ok(code)
}

fn run_reapply() -> Result<i32, String> {
    ensure_state_layout()?;
    let lock = acquire_operation_lock()?;
    let store = StateStore::new();
    let snapshot = load_state_preflight(&store, false)?;
    let stored = snapshot.settings;
    if !stored.auto_reapply {
        println!("auto_reapply=0");
        println!("controller_action=reapply");
        print!(
            "{}",
            render_operation_contract(
                OperationKind::Mutation,
                OperationResult::Success,
                false,
                Some(OperationState::NotStarted),
            )
        );
        return Ok(0);
    }
    let plan = build_reapply_plan(&stored);
    // AIDL policy XML is applied through its own validated adapter.  Release
    // the batch lock before replaying so each adapter operation can use the
    // same flock and common audit pipeline; the traditional path below keeps
    // the upstream coalesced-restart behavior unchanged.
    if aidl_backend::active_is_aidl()
        && plan
            .iter()
            .any(|action| matches!(action, domain::ReapplyAction::Policy(_)))
    {
        drop(lock);
        let mut applied = 0;
        for action in plan {
            let code = match action {
                domain::ReapplyAction::Policy(settings) => {
                    operation::run_operation(settings, Action::Apply)?
                }
                domain::ReapplyAction::Extra(extra) => operation::run_extra(extra)?,
            };
            if code != 0 {
                return Ok(code);
            }
            applied += 1;
        }
        println!("reapplied={applied}");
        return Ok(0);
    }
    let applied = plan.len();
    let code = run_reapply_batch_locked(&plan, &lock)?;
    if code != 0 {
        return Ok(code);
    }
    println!("reapplied={applied}");
    Ok(0)
}

#[cfg(test)]
mod tests;
