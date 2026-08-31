mod android;
mod catalog;
mod cli;
mod domain;
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
    render_operation_contract, run_extra, run_operation, run_reapply_batch_locked, OperationKind,
    OperationResult, OperationState,
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
        ControllerCommand::Extra(action) => run_extra(action),
        ControllerCommand::SetAutoReapply(enabled) => run_settings_command(enabled),
        ControllerCommand::Reapply => run_reapply(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationPreflight {
    action: String,
    kind: OperationKind,
}

pub(crate) fn operation_preflight(args: &[String]) -> Option<OperationPreflight> {
    match args.get(1).map(String::as_str) {
        Some("apply" | "reset" | "reapply") => Some(OperationPreflight {
            action: args[1].clone(),
            kind: OperationKind::Mutation,
        }),
        Some("settings") => Some(OperationPreflight {
            action: match (
                args.get(2).map(String::as_str),
                args.get(3).map(String::as_str),
            ) {
                (Some("auto-reapply"), _) => "settings-auto-reapply".to_string(),
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
