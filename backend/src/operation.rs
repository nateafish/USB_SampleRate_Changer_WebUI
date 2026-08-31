use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::android::{bluetooth_a2dp_state, namespace_info, verify_a2dp_route, A2dpState};
use crate::domain::{Action, ExtraAction, NamespaceInfo, ReapplyAction, Settings};
use crate::paths::{atomic_write, ensure_state_layout, log_root, module_dir, state_root, CORE_DIR};
use crate::process::{execute_stdin, execute_stdin_with_lease, ExecutionResult};
use crate::scripts::{
    extra_command_summaries, policy_command_summary, reapply_command_summaries,
    render_extra_restart_script, render_extra_script, render_extra_status_script,
    render_policy_script, render_reapply_restart_script, render_reapply_steps,
    render_reapply_verifications, validate_reapply_plan, validate_settings_for_action,
    ReapplyScriptStep,
};
use crate::state::{
    persist_extra_settings, reset_policy_settings, save_policy_settings, StateSnapshot, StateStore,
};

pub(crate) struct OperationLock {
    _file: fs::File,
}

pub(crate) fn load_state_preflight(
    store: &StateStore,
    allow_degraded_recovery: bool,
) -> Result<StateSnapshot, String> {
    let snapshot = store.load();
    if let Some(reason) = snapshot.health.reason() {
        if !allow_degraded_recovery {
            let safe_reason = reason.replace(['\n', '\r'], " ");
            println!("state_degraded=1");
            println!("state_degraded_reason={safe_reason}");
            return Err(format!("state is degraded: {safe_reason}"));
        }
    }
    Ok(snapshot)
}

pub(crate) fn execute_after_state_preflight<T, Execute>(
    store: &StateStore,
    allow_degraded_recovery: bool,
    execute: Execute,
) -> Result<T, String>
where
    Execute: FnOnce() -> Result<T, String>,
{
    load_state_preflight(store, allow_degraded_recovery)?;
    execute()
}

const EXECUTION_OUTPUT_LIMIT: usize = 64 * 1024;
const POLICY_TIMEOUT: Duration = Duration::from_secs(20);
const EXTRA_TIMEOUT: Duration = Duration::from_secs(15);
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(30);
const VERIFICATION_TIMEOUT: Duration = Duration::from_secs(5);
const REAPPLY_TIMEOUT: Duration = Duration::from_secs(60);
const REAPPLY_RESTART_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
pub(crate) struct ReapplyTimeouts {
    pub(crate) policy: Duration,
    pub(crate) extra: Duration,
    pub(crate) restart: Duration,
    pub(crate) verification: Duration,
    pub(crate) total: Duration,
}

const REAPPLY_TIMEOUTS: ReapplyTimeouts = ReapplyTimeouts {
    policy: POLICY_TIMEOUT,
    extra: EXTRA_TIMEOUT,
    restart: REAPPLY_RESTART_TIMEOUT,
    verification: VERIFICATION_TIMEOUT,
    total: REAPPLY_TIMEOUT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Query,
    Mutation,
}

impl OperationKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Mutation => "mutation",
        }
    }
}

impl OperationLock {
    pub(crate) fn lease(&self) -> Result<fs::File, String> {
        self._file
            .try_clone()
            .map_err(|error| format!("cannot retain operation lock for child process: {error}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationResult {
    Success,
    Failed,
    TimedOut,
    NotStarted,
}

impl OperationResult {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::NotStarted => "not_started",
        }
    }
}

/// How much of an operation may have reached the Android system. The legacy
/// boolean `operation_applied` remains available for older callers and keeps
/// its historical success-only meaning. Mutation callers should use the
/// detailed `operation_state`; queries use `operation_result` and do not emit
/// a synthetic mutation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationState {
    NotStarted,
    Applied,
    PartiallyApplied,
    PossiblyApplied,
}

impl OperationState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Applied => "applied",
            Self::PartiallyApplied => "partially_applied",
            Self::PossiblyApplied => "possibly_applied",
        }
    }

    fn legacy_applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

pub(crate) fn run_extra(action: ExtraAction) -> Result<i32, String> {
    let module_dir = module_dir()?;
    let script_path = module_dir
        .join(CORE_DIR)
        .join("extras")
        .join(action.script());
    if !script_path.is_file() {
        return Err(format!(
            "extras script not found: {}",
            script_path.display()
        ));
    }
    ensure_state_layout()?;
    let lock = acquire_operation_lock()?;
    let state_store = StateStore::new();
    let tool = action.tool();
    let generated = render_extra_script(&script_path, &action);
    let business_commands = extra_command_summaries(&script_path, &action);

    let kind = if action.is_query() {
        OperationKind::Query
    } else {
        OperationKind::Mutation
    };
    let a2dp_state = (kind == OperationKind::Mutation && action.requires_a2dp_post_check())
        .then(bluetooth_a2dp_state);
    let namespace = namespace_info();
    let timeout = if matches!(action, ExtraAction::Diagnose { .. }) {
        DIAGNOSTIC_TIMEOUT
    } else {
        EXTRA_TIMEOUT
    };
    let (route, command) = script_runner(&namespace);
    let (execution, phased_progress) =
        if kind == OperationKind::Query || !action.requires_audio_restart() {
            let execution = if kind == OperationKind::Query {
                execute_script(&generated, &namespace, timeout)?
            } else {
                execute_after_state_preflight(&state_store, false, || {
                    execute_script_locked(&generated, &namespace, timeout, &lock)
                })?
            };
            (execution, None)
        } else {
            load_state_preflight(&state_store, false)?;
            let restart = render_extra_restart_script(&script_path, &action)
                .expect("audio-restarting Extra must have a restart phase");
            let verification = render_extra_status_script(&script_path, &action);
            let phased = execute_phased_mutation(
                &generated,
                &restart,
                verification.as_deref(),
                &namespace,
                &lock,
            );
            (phased.execution, Some(phased.progress))
        };
    let label = format!("extra-{tool}");
    finalize_operation(
        OperationContext {
            action: &label,
            kind,
            route,
            command: &command,
            business_commands: &business_commands,
            script: &generated,
            a2dp_state,
            persistence: Persistence::Extra(&action),
            reapply_progress: None,
            phased_progress: phased_progress.as_ref(),
        },
        execution,
    )
}

pub(crate) fn run_operation(settings: Settings, action: Action) -> Result<i32, String> {
    let module_dir = module_dir()?;
    validate_settings_for_action(&settings, &module_dir, action)?;
    ensure_state_layout()?;
    let lock = acquire_operation_lock()?;
    let state_store = StateStore::new();

    let generated = render_policy_script(&settings, &module_dir, action);
    let business_commands = vec![policy_command_summary(&settings, &module_dir, action)];

    let a2dp_state = Some(bluetooth_a2dp_state());
    let namespace = namespace_info();
    let (route, command) = script_runner(&namespace);
    let execution =
        execute_after_state_preflight(&state_store, matches!(action, Action::Reset), || {
            execute_script_locked(&generated, &namespace, POLICY_TIMEOUT, &lock)
        })?;
    let persistence = match action {
        Action::Apply => Persistence::PolicyApply(&settings),
        Action::Reset => Persistence::PolicyReset,
    };
    finalize_operation(
        OperationContext {
            action: action.label(),
            kind: OperationKind::Mutation,
            route,
            command: &command,
            business_commands: &business_commands,
            script: &generated,
            a2dp_state,
            persistence,
            reapply_progress: None,
            phased_progress: None,
        },
        execution,
    )
}

pub(crate) fn run_reapply_batch_locked(
    plan: &[ReapplyAction],
    lock: &OperationLock,
) -> Result<i32, String> {
    let module_dir = module_dir()?;
    validate_reapply_plan(plan, &module_dir)?;

    let steps = render_reapply_steps(plan, &module_dir);
    let restart = render_reapply_restart_script(plan, &module_dir);
    let verifications = render_reapply_verifications(plan, &module_dir);
    let generated = reapply_script_bundle(&steps, restart.as_deref(), &verifications);
    let business_commands = reapply_command_summaries(plan, &module_dir);

    let needs_a2dp_post_check = plan.iter().any(ReapplyAction::requires_a2dp_post_check);
    let a2dp_state = needs_a2dp_post_check.then(bluetooth_a2dp_state);
    let namespace = namespace_info();
    let (route, command) = script_runner(&namespace);
    let reapply = execute_reapply_steps(
        plan,
        &steps,
        restart.as_deref(),
        &verifications,
        &namespace,
        lock,
    );
    finalize_operation(
        OperationContext {
            action: "reapply",
            kind: OperationKind::Mutation,
            route,
            command: &command,
            business_commands: &business_commands,
            script: &generated,
            a2dp_state,
            persistence: Persistence::NotRequired,
            reapply_progress: Some(&reapply.progress),
            phased_progress: None,
        },
        reapply.execution,
    )
}

pub(crate) fn finish_internal_mutation(
    action: &str,
    business_command: String,
) -> Result<i32, String> {
    let output = Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: b"controller_upstream_started=1\n".to_vec(),
    };
    let business_commands = vec![business_command];
    finalize_operation(
        OperationContext {
            action,
            kind: OperationKind::Mutation,
            route: "controller-internal",
            command: "internal state mutation",
            business_commands: &business_commands,
            script: "",
            a2dp_state: None,
            persistence: Persistence::AlreadyPersisted,
            reapply_progress: None,
            phased_progress: None,
        },
        ExecutionResult {
            output,
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
        },
    )
}

fn reapply_script_bundle(
    steps: &[ReapplyScriptStep],
    restart: Option<&str>,
    verifications: &[ReapplyScriptStep],
) -> String {
    let mut bundle = String::new();
    for step in steps {
        if !bundle.is_empty() {
            bundle.push_str("\n# controller-reapply-step-boundary\n");
        }
        bundle.push_str(&step.script);
    }
    if let Some(restart) = restart {
        if !bundle.is_empty() {
            bundle.push_str("\n# controller-reapply-restart-boundary\n");
        }
        bundle.push_str(restart);
    }
    for verification in verifications {
        if !bundle.is_empty() {
            bundle.push_str("\n# controller-reapply-verification-boundary\n");
        }
        bundle.push_str(&verification.script);
    }
    bundle
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProgressState {
    Pending,
    Running,
    Succeeded,
    Failed(i32),
    TimedOut { total_budget: bool },
    StartFailed,
    SkippedPriorFailure,
    SkippedTotalTimeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PhasedMutationProgress {
    pub(crate) business: ProgressState,
    pub(crate) restart: Option<ProgressState>,
    pub(crate) verification: Option<ProgressState>,
}

impl PhasedMutationProgress {
    pub(crate) fn operation_state(&self) -> OperationState {
        if self.business.succeeded()
            && self.restart.as_ref().is_none_or(ProgressState::succeeded)
            && self
                .verification
                .as_ref()
                .is_none_or(ProgressState::succeeded)
        {
            OperationState::Applied
        } else if self.business.succeeded() {
            OperationState::PartiallyApplied
        } else if self.business.started() {
            OperationState::PossiblyApplied
        } else {
            OperationState::NotStarted
        }
    }

    pub(crate) fn persistence_ready(&self) -> bool {
        self.business.succeeded()
    }

    fn post_check_ready(&self) -> bool {
        self.restart.as_ref().is_none_or(ProgressState::succeeded)
            && self
                .verification
                .as_ref()
                .is_none_or(ProgressState::succeeded)
    }

    fn render_markers(&self) -> String {
        let mut markers = String::new();
        render_named_phase(&mut markers, "business", &self.business);
        if let Some(restart) = &self.restart {
            render_named_phase(&mut markers, "restart", restart);
        }
        if let Some(verification) = &self.verification {
            render_named_phase(&mut markers, "verification", verification);
        }
        markers
    }
}

fn render_named_phase(markers: &mut String, name: &str, state: &ProgressState) {
    let value = match state {
        ProgressState::Pending => "pending".to_string(),
        ProgressState::Running => "running".to_string(),
        ProgressState::Succeeded => "succeeded".to_string(),
        ProgressState::Failed(code) => format!("failed:{code}"),
        ProgressState::TimedOut { .. } => "timed_out".to_string(),
        ProgressState::StartFailed => "start_failed".to_string(),
        ProgressState::SkippedPriorFailure => "skipped_prior_failure".to_string(),
        ProgressState::SkippedTotalTimeout => "skipped_total_timeout".to_string(),
    };
    markers.push_str(&format!("mutation_phase_{name}={value}\n"));
}

impl ProgressState {
    fn started(&self) -> bool {
        matches!(
            self,
            Self::Running | Self::Succeeded | Self::Failed(_) | Self::TimedOut { .. }
        )
    }

    fn succeeded(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StepProgress {
    pub(crate) number: usize,
    pub(crate) label: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) state: ProgressState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestartProgress {
    pub(crate) timeout_seconds: u64,
    pub(crate) state: ProgressState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReapplyProgress {
    pub(crate) total_timeout_seconds: u64,
    pub(crate) total_timed_out: bool,
    pub(crate) steps: Vec<StepProgress>,
    pub(crate) restart: Option<RestartProgress>,
    pub(crate) verifications: Vec<StepProgress>,
}

impl ReapplyProgress {
    pub(crate) fn operation_state(&self) -> OperationState {
        let states = self
            .steps
            .iter()
            .map(|step| &step.state)
            .chain(self.restart.iter().map(|restart| &restart.state))
            .chain(self.verifications.iter().map(|step| &step.state));
        let has_work =
            !self.steps.is_empty() || self.restart.is_some() || !self.verifications.is_empty();
        let all_succeeded = has_work
            && self.steps.iter().all(|step| step.state.succeeded())
            && self
                .restart
                .as_ref()
                .is_none_or(|restart| restart.state.succeeded())
            && self.verifications.iter().all(|step| step.state.succeeded());
        if all_succeeded {
            return OperationState::Applied;
        }
        let states = states.collect::<Vec<_>>();
        if states.iter().any(|state| state.succeeded()) {
            OperationState::PartiallyApplied
        } else if states.iter().any(|state| state.started()) {
            OperationState::PossiblyApplied
        } else {
            OperationState::NotStarted
        }
    }

    fn post_check_ready(&self) -> bool {
        self.restart
            .as_ref()
            .is_none_or(|restart| restart.state.succeeded())
            && self
                .verifications
                .iter()
                .all(|verification| verification.state.succeeded())
    }

    fn render_markers(&self) -> String {
        let mut markers = format!(
            "batch_total_timeout_seconds={}\n",
            self.total_timeout_seconds
        );
        for step in &self.steps {
            markers.push_str(&format!("batch_step={}:{}\n", step.number, step.label));
            markers.push_str(&format!(
                "batch_step_timeout_seconds={}:{}\n",
                step.number, step.timeout_seconds
            ));
            render_step_state(&mut markers, step.number, &step.state);
        }
        match &self.restart {
            None => markers.push_str("batch_restart=not-required\n"),
            Some(restart) if restart.state == ProgressState::SkippedPriorFailure => {
                render_restart_state(&mut markers, &restart.state);
            }
            Some(restart) => {
                markers.push_str("batch_restart=audioserver\n");
                markers.push_str(&format!(
                    "batch_restart_timeout_seconds={}\n",
                    restart.timeout_seconds
                ));
                render_restart_state(&mut markers, &restart.state);
            }
        }
        for verification in &self.verifications {
            markers.push_str(&format!(
                "batch_verification={}:{}\n",
                verification.number, verification.label
            ));
            markers.push_str(&format!(
                "batch_verification_timeout_seconds={}:{}\n",
                verification.number, verification.timeout_seconds
            ));
            render_verification_state(&mut markers, verification.number, &verification.state);
        }
        markers.push_str(&format!(
            "batch_total_timed_out={}\n",
            if self.total_timed_out { 1 } else { 0 }
        ));
        markers
    }
}

fn render_verification_state(markers: &mut String, number: usize, state: &ProgressState) {
    match state {
        ProgressState::Pending => {}
        ProgressState::Running => {
            markers.push_str(&format!("batch_verification_started={number}\n"));
        }
        ProgressState::Succeeded => markers.push_str(&format!(
            "batch_verification_started={number}\nbatch_verification_timed_out={number}:0\nbatch_verification_exit={number}:0\n"
        )),
        ProgressState::Failed(code) => markers.push_str(&format!(
            "batch_verification_started={number}\nbatch_verification_timed_out={number}:0\nbatch_verification_exit={number}:{code}\n"
        )),
        ProgressState::TimedOut { total_budget } => {
            markers.push_str(&format!(
                "batch_verification_started={number}\nbatch_verification_timed_out={number}:1\nbatch_verification_exit={number}:124\n"
            ));
            markers.push_str(if *total_budget {
                "batch_timeout_scope=total\n"
            } else {
                "batch_timeout_scope=verification\n"
            });
        }
        ProgressState::StartFailed => markers.push_str(&format!(
            "batch_verification_start_failed={number}\nbatch_verification_exit={number}:125\n"
        )),
        ProgressState::SkippedPriorFailure => markers.push_str(&format!(
            "batch_verification_skipped={number}:prior-phase-failure-or-timeout\n"
        )),
        ProgressState::SkippedTotalTimeout => {
            markers.push_str(&format!(
                "batch_verification_skipped={number}:total-timeout\n"
            ));
            markers.push_str("batch_timeout_scope=total\n");
        }
    }
}

fn render_step_state(markers: &mut String, number: usize, state: &ProgressState) {
    match state {
        ProgressState::Pending => {}
        ProgressState::Running => markers.push_str(&format!("batch_step_started={number}\n")),
        ProgressState::Succeeded => markers.push_str(&format!(
            "batch_step_started={number}\nbatch_step_timed_out={number}:0\nbatch_step_exit={number}:0\n"
        )),
        ProgressState::Failed(code) => markers.push_str(&format!(
            "batch_step_started={number}\nbatch_step_timed_out={number}:0\nbatch_step_exit={number}:{code}\n"
        )),
        ProgressState::TimedOut { total_budget } => {
            markers.push_str(&format!(
                "batch_step_started={number}\nbatch_step_timed_out={number}:1\nbatch_step_exit={number}:124\n"
            ));
            if *total_budget {
                markers.push_str("batch_timeout_scope=total\n");
            } else {
                markers.push_str(&format!("batch_timeout_scope=step:{number}\n"));
            }
        }
        ProgressState::StartFailed => markers.push_str(&format!(
            "batch_step_start_failed={number}\nbatch_step_exit={number}:125\n"
        )),
        ProgressState::SkippedPriorFailure => {
            markers.push_str(&format!("batch_step_skipped={number}:prior-failure\n"));
        }
        ProgressState::SkippedTotalTimeout => {
            markers.push_str(&format!("batch_step_skipped={number}:total-timeout\n"));
            markers.push_str("batch_timeout_scope=total\n");
        }
    }
}

fn render_restart_state(markers: &mut String, state: &ProgressState) {
    match state {
        ProgressState::Pending => {}
        ProgressState::Running => markers.push_str("batch_restart_started=1\n"),
        ProgressState::Succeeded => {
            markers.push_str(
                "batch_restart_started=1\nbatch_restart_timed_out=0\nbatch_restart_exit=0\n",
            );
        }
        ProgressState::Failed(code) => markers.push_str(&format!(
            "batch_restart_started=1\nbatch_restart_timed_out=0\nbatch_restart_exit={code}\n"
        )),
        ProgressState::TimedOut { total_budget } => {
            markers.push_str(
                "batch_restart_started=1\nbatch_restart_timed_out=1\nbatch_restart_exit=124\n",
            );
            markers.push_str(if *total_budget {
                "batch_timeout_scope=total\n"
            } else {
                "batch_timeout_scope=restart\n"
            });
        }
        ProgressState::StartFailed => {
            markers.push_str("batch_restart_start_failed=1\nbatch_restart_exit=125\n");
        }
        ProgressState::SkippedPriorFailure => {
            markers.push_str(
                "batch_restart=skipped\nbatch_restart_skipped=prior-business-failure-or-timeout\n",
            );
        }
        ProgressState::SkippedTotalTimeout => {
            markers.push_str("batch_restart_skipped=total-timeout\nbatch_timeout_scope=total\n");
        }
    }
}

pub(crate) struct ReapplyExecution {
    pub(crate) execution: ExecutionResult,
    pub(crate) progress: ReapplyProgress,
}

pub(crate) struct PhasedMutationExecution {
    pub(crate) execution: ExecutionResult,
    pub(crate) progress: PhasedMutationProgress,
}

fn execute_phased_mutation(
    business: &str,
    restart: &str,
    verification: Option<&str>,
    namespace: &NamespaceInfo,
    lock: &OperationLock,
) -> PhasedMutationExecution {
    execute_phased_mutation_with(business, restart, verification, |script, timeout| {
        execute_script_locked(script, namespace, timeout, lock)
    })
}

pub(crate) fn execute_phased_mutation_with<Executor>(
    business: &str,
    restart: &str,
    verification: Option<&str>,
    mut executor: Executor,
) -> PhasedMutationExecution
where
    Executor: FnMut(&str, Duration) -> Result<ExecutionResult, String>,
{
    let mut batch = ReapplyBatchOutput::new();
    let mut progress = PhasedMutationProgress {
        business: ProgressState::Running,
        restart: Some(ProgressState::Pending),
        verification: verification.map(|_| ProgressState::Pending),
    };
    let mut failure_code = 0;
    let mut timed_out = false;

    match executor(business, EXTRA_TIMEOUT) {
        Ok(execution) => {
            let code = execution_code(&execution);
            timed_out |= execution.timed_out;
            progress.business = progress_state(&execution, code, false);
            batch.append_execution(execution);
            if code != 0 {
                failure_code = code;
            }
        }
        Err(error) => {
            failure_code = 125;
            progress.business = ProgressState::StartFailed;
            batch.error(format!("cannot start mutation business phase: {error}\n"));
        }
    }

    if progress.business.succeeded() {
        *progress.restart.as_mut().expect("restart phase is present") = ProgressState::Running;
        match executor(restart, REAPPLY_RESTART_TIMEOUT) {
            Ok(execution) => {
                let code = execution_code(&execution);
                timed_out |= execution.timed_out;
                *progress.restart.as_mut().expect("restart phase is present") =
                    progress_state(&execution, code, false);
                batch.append_execution(execution);
                if code != 0 {
                    failure_code = code;
                }
            }
            Err(error) => {
                failure_code = 125;
                *progress.restart.as_mut().expect("restart phase is present") =
                    ProgressState::StartFailed;
                batch.error(format!("cannot start mutation restart phase: {error}\n"));
            }
        }
    } else {
        *progress.restart.as_mut().expect("restart phase is present") =
            ProgressState::SkippedPriorFailure;
    }

    if let (Some(script), Some(state)) = (verification, progress.verification.as_mut()) {
        if progress
            .restart
            .as_ref()
            .is_some_and(ProgressState::succeeded)
        {
            *state = ProgressState::Running;
            match executor(script, VERIFICATION_TIMEOUT) {
                Ok(execution) => {
                    let code = execution_code(&execution);
                    timed_out |= execution.timed_out;
                    *state = progress_state(&execution, code, false);
                    batch.append_execution(execution);
                    if code != 0 {
                        failure_code = code;
                    }
                }
                Err(error) => {
                    failure_code = 125;
                    *state = ProgressState::StartFailed;
                    batch.error(format!(
                        "cannot start mutation verification phase: {error}\n"
                    ));
                }
            }
        } else {
            *state = ProgressState::SkippedPriorFailure;
        }
    }

    batch.marker(progress.render_markers());
    PhasedMutationExecution {
        execution: batch.finish(failure_code, timed_out),
        progress,
    }
}

fn execution_code(execution: &ExecutionResult) -> i32 {
    if execution.timed_out {
        124
    } else {
        execution.output.status.code().unwrap_or(1)
    }
}

fn progress_state(execution: &ExecutionResult, code: i32, total_budget: bool) -> ProgressState {
    if execution.timed_out {
        ProgressState::TimedOut { total_budget }
    } else if code == 0 {
        ProgressState::Succeeded
    } else {
        ProgressState::Failed(code)
    }
}

fn execute_reapply_steps(
    plan: &[ReapplyAction],
    steps: &[ReapplyScriptStep],
    restart: Option<&str>,
    verifications: &[ReapplyScriptStep],
    namespace: &NamespaceInfo,
    lock: &OperationLock,
) -> ReapplyExecution {
    let started_at = Instant::now();
    execute_reapply_steps_with(
        plan,
        steps,
        restart,
        verifications,
        REAPPLY_TIMEOUTS,
        || started_at.elapsed(),
        |script, timeout| execute_script_locked(script, namespace, timeout, lock),
    )
}

pub(crate) fn execute_reapply_steps_with<Elapsed, Executor>(
    plan: &[ReapplyAction],
    steps: &[ReapplyScriptStep],
    restart: Option<&str>,
    verifications: &[ReapplyScriptStep],
    timeouts: ReapplyTimeouts,
    mut elapsed: Elapsed,
    mut executor: Executor,
) -> ReapplyExecution
where
    Elapsed: FnMut() -> Duration,
    Executor: FnMut(&str, Duration) -> Result<ExecutionResult, String>,
{
    debug_assert_eq!(plan.len(), steps.len());
    let mut batch = ReapplyBatchOutput::new();
    let mut failure_code = 0;
    let mut any_step_started = false;
    let mut any_timed_out = false;
    let mut total_timed_out = false;
    let mut progress = ReapplyProgress {
        total_timeout_seconds: timeouts.total.as_secs(),
        total_timed_out: false,
        steps: Vec::with_capacity(steps.len()),
        restart: restart.map(|_| RestartProgress {
            timeout_seconds: timeouts.restart.as_secs(),
            state: ProgressState::Pending,
        }),
        verifications: verifications
            .iter()
            .enumerate()
            .map(|(index, verification)| StepProgress {
                number: index + 1,
                label: verification.label.clone(),
                timeout_seconds: timeouts.verification.as_secs(),
                state: ProgressState::Pending,
            })
            .collect(),
    };

    for (index, (action, step)) in plan.iter().zip(steps).enumerate() {
        let number = index + 1;
        let configured_timeout = match action {
            ReapplyAction::Policy(_) => timeouts.policy,
            ReapplyAction::Extra(_) => timeouts.extra,
        };
        progress.steps.push(StepProgress {
            number,
            label: step.label.clone(),
            timeout_seconds: configured_timeout.as_secs(),
            state: ProgressState::Pending,
        });
        let state = &mut progress.steps[index].state;
        if failure_code != 0 {
            *state = ProgressState::SkippedPriorFailure;
            continue;
        }

        let remaining = timeouts.total.saturating_sub(elapsed());
        if remaining.is_zero() {
            failure_code = 124;
            any_timed_out = true;
            total_timed_out = true;
            *state = ProgressState::SkippedTotalTimeout;
            continue;
        }
        let effective_timeout = configured_timeout.min(remaining);
        let limited_by_total = effective_timeout < configured_timeout;
        *state = ProgressState::Running;
        match executor(&step.script, effective_timeout) {
            Ok(execution) => {
                any_step_started = true;
                let step_code = if execution.timed_out {
                    124
                } else {
                    execution.output.status.code().unwrap_or(1)
                };
                if execution.timed_out {
                    any_timed_out = true;
                    if limited_by_total {
                        total_timed_out = true;
                    }
                    *state = ProgressState::TimedOut {
                        total_budget: limited_by_total,
                    };
                } else if step_code == 0 {
                    *state = ProgressState::Succeeded;
                } else {
                    *state = ProgressState::Failed(step_code);
                }
                batch.append_execution(execution);
                if step_code != 0 {
                    failure_code = step_code;
                }
            }
            Err(error) => {
                failure_code = 125;
                *state = ProgressState::StartFailed;
                batch.error(format!("cannot start reapply step {number}: {error}\n"));
            }
        }
    }

    match restart {
        None => {}
        Some(_) if any_timed_out => {
            progress.restart.as_mut().unwrap().state = ProgressState::SkippedPriorFailure;
        }
        Some(_) if !any_step_started => {
            progress.restart.as_mut().unwrap().state = ProgressState::SkippedPriorFailure;
        }
        Some(restart_script) => {
            let restart_state = &mut progress.restart.as_mut().unwrap().state;
            let remaining = timeouts.total.saturating_sub(elapsed());
            if remaining.is_zero() {
                any_timed_out = true;
                total_timed_out = true;
                if failure_code == 0 {
                    failure_code = 124;
                }
                *restart_state = ProgressState::SkippedTotalTimeout;
            } else {
                let effective_timeout = timeouts.restart.min(remaining);
                let limited_by_total = effective_timeout < timeouts.restart;
                *restart_state = ProgressState::Running;
                match executor(restart_script, effective_timeout) {
                    Ok(execution) => {
                        let restart_code = if execution.timed_out {
                            124
                        } else {
                            execution.output.status.code().unwrap_or(1)
                        };
                        if execution.timed_out {
                            any_timed_out = true;
                            if limited_by_total {
                                total_timed_out = true;
                            }
                            *restart_state = ProgressState::TimedOut {
                                total_budget: limited_by_total,
                            };
                        } else if restart_code == 0 {
                            *restart_state = ProgressState::Succeeded;
                        } else {
                            *restart_state = ProgressState::Failed(restart_code);
                        }
                        batch.append_execution(execution);
                        if failure_code == 0 && restart_code != 0 {
                            failure_code = restart_code;
                        }
                    }
                    Err(error) => {
                        if failure_code == 0 {
                            failure_code = 125;
                        }
                        *restart_state = ProgressState::StartFailed;
                        batch.error(format!("cannot start reapply restart step: {error}\n"));
                    }
                }
            }
        }
    }

    let restart_succeeded = progress
        .restart
        .as_ref()
        .is_none_or(|restart| restart.state.succeeded());
    for (index, verification) in verifications.iter().enumerate() {
        let state = &mut progress.verifications[index].state;
        if !restart_succeeded || any_timed_out {
            *state = ProgressState::SkippedPriorFailure;
            continue;
        }
        let remaining = timeouts.total.saturating_sub(elapsed());
        if remaining.is_zero() {
            any_timed_out = true;
            total_timed_out = true;
            if failure_code == 0 {
                failure_code = 124;
            }
            *state = ProgressState::SkippedTotalTimeout;
            continue;
        }
        let effective_timeout = timeouts.verification.min(remaining);
        let limited_by_total = effective_timeout < timeouts.verification;
        *state = ProgressState::Running;
        match executor(&verification.script, effective_timeout) {
            Ok(execution) => {
                let code = execution_code(&execution);
                any_timed_out |= execution.timed_out;
                if execution.timed_out && limited_by_total {
                    total_timed_out = true;
                }
                *state = progress_state(&execution, code, limited_by_total);
                batch.append_execution(execution);
                if failure_code == 0 && code != 0 {
                    failure_code = code;
                }
            }
            Err(error) => {
                if failure_code == 0 {
                    failure_code = 125;
                }
                *state = ProgressState::StartFailed;
                batch.error(format!(
                    "cannot start reapply verification {}: {error}\n",
                    index + 1
                ));
            }
        }
    }

    progress.total_timed_out = total_timed_out;
    batch.marker(progress.render_markers());
    ReapplyExecution {
        execution: batch.finish(failure_code, any_timed_out),
        progress,
    }
}

struct ReapplyBatchOutput {
    control: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl ReapplyBatchOutput {
    fn new() -> Self {
        Self {
            control: String::new(),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn marker(&mut self, marker: impl AsRef<str>) {
        self.control.push_str(marker.as_ref());
    }

    fn error(&mut self, error: String) {
        append_capped(
            &mut self.stderr,
            error.as_bytes(),
            EXECUTION_OUTPUT_LIMIT,
            &mut self.stderr_truncated,
        );
    }

    fn append_execution(&mut self, execution: ExecutionResult) {
        self.stdout_truncated |= execution.stdout_truncated;
        self.stderr_truncated |= execution.stderr_truncated;
        append_reapply_payload(
            &mut self.stdout,
            &execution.output.stdout,
            EXECUTION_OUTPUT_LIMIT,
            &mut self.stdout_truncated,
        );
        append_reapply_payload(
            &mut self.stderr,
            &execution.output.stderr,
            EXECUTION_OUTPUT_LIMIT,
            &mut self.stderr_truncated,
        );
    }

    fn finish(mut self, code: i32, timed_out: bool) -> ExecutionResult {
        let mut stdout = self.control.into_bytes();
        append_capped(
            &mut stdout,
            &self.stdout,
            EXECUTION_OUTPUT_LIMIT,
            &mut self.stdout_truncated,
        );
        if stdout.len() > EXECUTION_OUTPUT_LIMIT {
            stdout.truncate(EXECUTION_OUTPUT_LIMIT);
            self.stdout_truncated = true;
        }
        ExecutionResult {
            output: Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout,
                stderr: self.stderr,
            },
            timed_out,
            stdout_truncated: self.stdout_truncated,
            stderr_truncated: self.stderr_truncated,
        }
    }
}

fn append_reapply_payload(target: &mut Vec<u8>, bytes: &[u8], limit: usize, truncated: &mut bool) {
    // batch_* is a reserved controller namespace. Drop upstream lookalikes so
    // every public progress marker is rendered from ReapplyProgress alone.
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.starts_with(b"batch_") {
            append_capped(target, line, limit, truncated);
        }
    }
}

fn append_capped(target: &mut Vec<u8>, bytes: &[u8], limit: usize, truncated: &mut bool) {
    let available = limit.saturating_sub(target.len());
    target.extend_from_slice(&bytes[..bytes.len().min(available)]);
    if bytes.len() > available {
        *truncated = true;
    }
}

enum Persistence<'a> {
    PolicyApply(&'a Settings),
    PolicyReset,
    Extra(&'a ExtraAction),
    AlreadyPersisted,
    NotRequired,
}

impl Persistence<'_> {
    fn persist(self) -> Result<&'static str, String> {
        match self {
            Self::PolicyApply(settings) => {
                save_policy_settings(settings)?;
                Ok("ok")
            }
            Self::PolicyReset => {
                reset_policy_settings()?;
                Ok("ok")
            }
            Self::Extra(action) if !action.persists_state() => Ok("not-required"),
            Self::Extra(action) => {
                persist_extra_settings(action)?;
                Ok("ok")
            }
            Self::AlreadyPersisted => Ok("ok"),
            Self::NotRequired => Ok("not-required"),
        }
    }
}

struct OperationContext<'a> {
    action: &'a str,
    kind: OperationKind,
    route: &'a str,
    command: &'a str,
    business_commands: &'a [String],
    script: &'a str,
    a2dp_state: Option<A2dpState>,
    persistence: Persistence<'a>,
    reapply_progress: Option<&'a ReapplyProgress>,
    phased_progress: Option<&'a PhasedMutationProgress>,
}

/// Complete the common post-execution pipeline.  Upstream failure, A2DP
/// verification, state persistence, and audit persistence remain distinct so
/// callers can tell whether the Android system was already modified.
fn finalize_operation(
    context: OperationContext<'_>,
    execution: ExecutionResult,
) -> Result<i32, String> {
    let OperationContext {
        action,
        kind,
        route,
        command,
        business_commands,
        script,
        a2dp_state,
        persistence,
        reapply_progress,
        phased_progress,
    } = context;
    let ExecutionResult {
        mut output,
        timed_out,
        stdout_truncated,
        stderr_truncated,
    } = execution;
    if timed_out {
        output
            .stderr
            .extend_from_slice(b"controller execution timed out\n");
    }
    let (upstream_status, upstream_code) = classify_upstream(timed_out, output.status.code());
    let operation_state = (kind == OperationKind::Mutation).then(|| {
        reapply_progress.map_or_else(
            || {
                phased_progress.map_or_else(
                    || classify_operation_state(&output, timed_out, upstream_code),
                    PhasedMutationProgress::operation_state,
                )
            },
            ReapplyProgress::operation_state,
        )
    });
    let operation_applied = operation_state.is_some_and(OperationState::legacy_applied);
    let operation_may_have_applied =
        operation_state.is_some_and(|state| !matches!(state, OperationState::NotStarted));
    let post_check_ready = phased_progress
        .map(PhasedMutationProgress::post_check_ready)
        .or_else(|| reapply_progress.map(ReapplyProgress::post_check_ready))
        .unwrap_or(operation_may_have_applied);
    let upstream_succeeded = !timed_out && upstream_code == 0;
    let operation_started = upstream_succeeded
        || upstream_started(&output)
        || operation_state.is_some_and(|state| !matches!(state, OperationState::NotStarted));
    let mut final_code = upstream_code;

    let post_check = if !operation_may_have_applied || !post_check_ready {
        "skipped"
    } else if matches!(a2dp_state, Some(A2dpState::Unknown)) {
        output
            .stdout
            .extend_from_slice(b"bluetooth_a2dp_state=unknown\n");
        "unknown"
    } else if matches!(a2dp_state, Some(A2dpState::Connected)) {
        output
            .stdout
            .extend_from_slice(b"bluetooth_a2dp_before=1\n");
        match verify_a2dp_route() {
            Ok(result) => {
                output.stdout.extend_from_slice(
                    format!("bluetooth_route_check={result}\nbluetooth_a2dp_after=1\n").as_bytes(),
                );
                "ok"
            }
            Err(error) => {
                final_code = 72;
                output.stderr.extend_from_slice(
                    format!("Bluetooth A2DP route verification failed: {error}\n").as_bytes(),
                );
                "a2dp-route-failed"
            }
        }
    } else if a2dp_state.is_none() {
        "not-required"
    } else {
        "not-connected"
    };

    let persistence_allowed = persistence_allowed_with_progress(
        kind,
        upstream_succeeded,
        operation_state,
        phased_progress.map(PhasedMutationProgress::persistence_ready),
    );
    let state_persist = if persistence_allowed {
        match persistence.persist() {
            Ok(status) => status,
            Err(error) => {
                if final_code == 0 {
                    final_code = 73;
                }
                output.stderr.extend_from_slice(
                    match kind {
                        OperationKind::Mutation => format!(
                            "System was modified but controller state persistence failed: {error}\n"
                        ),
                        OperationKind::Query => {
                            format!("Query succeeded but controller preference persistence failed: {error}\n")
                        }
                    }
                    .as_bytes(),
                );
                "failed"
            }
        }
    } else {
        "skipped"
    };
    let operation_result = classify_operation_result(final_code, timed_out, operation_started);

    let audit_result = write_operation_log(&AuditRecord {
        action,
        kind,
        result: operation_result,
        route,
        code: final_code,
        command,
        business_commands,
        script,
        output: &output,
        timed_out,
        stdout_truncated,
        stderr_truncated,
        upstream_status,
        post_check,
        state_persist,
        operation_applied,
        operation_state,
    });
    let audit_status = if let Err(error) = &audit_result {
        output
            .stderr
            .extend_from_slice(format!("Audit log write failed: {error}\n").as_bytes());
        "failed"
    } else {
        "ok"
    };

    println!("controller_action={action}");
    print!(
        "{}",
        render_operation_contract(kind, operation_result, operation_applied, operation_state)
    );
    println!("controller_route={route}");
    println!("controller_exit={final_code}");
    println!("upstream_status={upstream_status}");
    println!("upstream_exit={upstream_code}");
    println!("post_check={post_check}");
    println!("state_persist={state_persist}");
    println!("audit_status={audit_status}");
    println!("timed_out={}", if timed_out { 1 } else { 0 });
    println!("stdout_truncated={}", if stdout_truncated { 1 } else { 0 });
    println!("stderr_truncated={}", if stderr_truncated { 1 } else { 0 });
    println!(
        "command_log={}",
        log_root().join("last-command.log").display()
    );
    println!("script_summary={}", script_summary(script));
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(final_code)
}

#[cfg(test)]
pub(crate) fn persistence_allowed(
    kind: OperationKind,
    upstream_succeeded: bool,
    operation_state: Option<OperationState>,
) -> bool {
    persistence_allowed_with_progress(kind, upstream_succeeded, operation_state, None)
}

fn persistence_allowed_with_progress(
    kind: OperationKind,
    upstream_succeeded: bool,
    operation_state: Option<OperationState>,
    phased_business_succeeded: Option<bool>,
) -> bool {
    match kind {
        OperationKind::Query => upstream_succeeded,
        OperationKind::Mutation => {
            phased_business_succeeded.unwrap_or(operation_state == Some(OperationState::Applied))
        }
    }
}

pub(crate) fn render_operation_contract(
    kind: OperationKind,
    result: OperationResult,
    operation_applied: bool,
    operation_state: Option<OperationState>,
) -> String {
    let mut fields = format!(
        "operation_kind={}\noperation_result={}\noperation_applied={}\n",
        kind.label(),
        result.label(),
        if operation_applied { 1 } else { 0 }
    );
    if kind == OperationKind::Mutation {
        fields.push_str(&format!(
            "operation_state={}\n",
            operation_state
                .expect("mutation operation contract requires operation_state")
                .label()
        ));
    }
    fields
}

pub(crate) fn classify_upstream(timed_out: bool, exit_code: Option<i32>) -> (&'static str, i32) {
    if timed_out {
        ("timeout", 124)
    } else {
        let code = exit_code.unwrap_or(1);
        (if code == 0 { "ok" } else { "failed" }, code)
    }
}

pub(crate) fn classify_operation_result(
    final_code: i32,
    timed_out: bool,
    operation_started: bool,
) -> OperationResult {
    if timed_out {
        OperationResult::TimedOut
    } else if !operation_started {
        OperationResult::NotStarted
    } else if final_code == 0 {
        OperationResult::Success
    } else {
        OperationResult::Failed
    }
}

/// Infer how much a single upstream mutation may have changed. Reapply never
/// uses text parsing here; its state comes exclusively from `ReapplyProgress`.
pub(crate) fn classify_operation_state(
    output: &Output,
    timed_out: bool,
    upstream_code: i32,
) -> OperationState {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if !timed_out && upstream_code == 0 {
        OperationState::Applied
    } else if combined
        .lines()
        .any(|line| line == "controller_upstream_started=1")
    {
        OperationState::PossiblyApplied
    } else {
        OperationState::NotStarted
    }
}

fn upstream_started(output: &Output) -> bool {
    [&output.stdout, &output.stderr].iter().any(|stream| {
        String::from_utf8_lossy(stream)
            .lines()
            .any(|line| line == "controller_upstream_started=1")
    })
}

pub(crate) fn acquire_operation_lock() -> Result<OperationLock, String> {
    let path = state_root().join("operation.lock");
    acquire_operation_lock_at(&path)
}

pub(crate) fn acquire_operation_lock_at(path: &std::path::Path) -> Result<OperationLock, String> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("cannot open operation lock {}: {error}", path.display()))?;

    // `flock` is associated with the open descriptor, so a process crash or
    // ordinary drop releases the lock without stale-file heuristics.
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    let result = unsafe { libc_flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if result == 0 {
        return Ok(OperationLock { _file: file });
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(11 | 35)) {
        Err("another audio operation is still running".to_string())
    } else {
        Err(format!("cannot acquire operation lock: {error}"))
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[cfg(unix)]
unsafe fn libc_flock(fd: i32, operation: i32) -> i32 {
    flock(fd, operation)
}

fn script_runner(namespace: &NamespaceInfo) -> (&'static str, String) {
    if namespace.matches() == Some(true) {
        ("current-namespace", "/system/bin/sh -s".to_string())
    } else {
        (
            "mount-master",
            "su --mount-master -c '/system/bin/sh -s'".to_string(),
        )
    }
}

fn execute_script(
    script: &str,
    namespace: &NamespaceInfo,
    timeout: Duration,
) -> Result<ExecutionResult, String> {
    if namespace.matches() == Some(true) {
        let mut process = Command::new("/system/bin/sh");
        process.arg("-s");
        let output = execute_stdin(&mut process, script, timeout, EXECUTION_OUTPUT_LIMIT)
            .map_err(|error| format!("cannot execute in current namespace: {error}"))?;
        return Ok(output);
    }

    let shell_command = "/system/bin/sh -s";
    let mut process = Command::new("su");
    process.args(["--mount-master", "-c", shell_command]);
    let output = execute_stdin(&mut process, script, timeout, EXECUTION_OUTPUT_LIMIT)
        .map_err(|error| format!("cannot execute mount-master shell: {error}"))?;
    Ok(output)
}

fn execute_script_locked(
    script: &str,
    namespace: &NamespaceInfo,
    timeout: Duration,
    lock: &OperationLock,
) -> Result<ExecutionResult, String> {
    let lease = lock.lease()?;
    if namespace.matches() == Some(true) {
        let mut process = Command::new("/system/bin/sh");
        process.arg("-s");
        return execute_stdin_with_lease(
            &mut process,
            script,
            timeout,
            EXECUTION_OUTPUT_LIMIT,
            lease,
        )
        .map_err(|error| format!("cannot execute in current namespace: {error}"));
    }

    let mut process = Command::new("su");
    process.args(["--mount-master", "-c", "/system/bin/sh -s"]);
    execute_stdin_with_lease(&mut process, script, timeout, EXECUTION_OUTPUT_LIMIT, lease)
        .map_err(|error| format!("cannot execute mount-master shell: {error}"))
}

/// A compact, non-executable description of the generated script.  The hash
/// is FNV-1a so it is deterministic without adding a crypto dependency.
pub(crate) fn script_summary(script: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in script.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(
        "bytes={};lines={};hash={hash:016x}",
        script.len(),
        script.lines().count()
    )
}

#[derive(Clone, Copy)]
struct AuditRecord<'a> {
    action: &'a str,
    kind: OperationKind,
    result: OperationResult,
    route: &'a str,
    code: i32,
    command: &'a str,
    business_commands: &'a [String],
    script: &'a str,
    output: &'a Output,
    timed_out: bool,
    stdout_truncated: bool,
    stderr_truncated: bool,
    upstream_status: &'a str,
    post_check: &'a str,
    state_persist: &'a str,
    operation_applied: bool,
    operation_state: Option<OperationState>,
}

fn write_operation_log(record: &AuditRecord<'_>) -> Result<(), String> {
    write_operation_log_at(record, &log_root())
}

fn write_operation_log_at(record: &AuditRecord<'_>, root: &std::path::Path) -> Result<(), String> {
    let AuditRecord {
        action,
        kind,
        result,
        route,
        code,
        command,
        business_commands,
        script,
        output,
        timed_out,
        stdout_truncated,
        stderr_truncated,
        upstream_status,
        post_check,
        state_persist,
        operation_applied,
        operation_state,
    } = *record;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let operation_state_field = operation_state
        .map(|state| format!("operation_state={}\n", state.label()))
        .unwrap_or_default();
    let status = (kind == OperationKind::Mutation).then(|| format!(
        "last_action={}\nlast_operation_kind={}\nlast_operation_result={}\nlast_route={route}\nlast_exit={code}\nlast_time={timestamp}\nlast_timed_out={}\nlast_stdout_truncated={}\nlast_stderr_truncated={}\nlast_upstream_status={upstream_status}\nlast_post_check={post_check}\nlast_state_persist={state_persist}\nlast_operation_applied={}\nlast_operation_state={}\nlast_audit_status=ok\n",
        action,
        kind.label(),
        result.label(),
        if timed_out { 1 } else { 0 },
        if stdout_truncated { 1 } else { 0 },
        if stderr_truncated { 1 } else { 0 },
        if operation_applied { 1 } else { 0 },
        operation_state
            .expect("mutation audit records must include operation_state")
            .label(),
    ));

    let commands = render_business_commands(business_commands);
    let log = format!(
        "time={timestamp}\naction={action}\noperation_kind={}\noperation_result={}\nroute={route}\nexit={code}\nupstream_status={upstream_status}\npost_check={post_check}\nstate_persist={state_persist}\noperation_applied={}\n{operation_state_field}audit_status=ok\ncommand={command}\nrunner_command={command}\n{commands}script_summary={}\ntimed_out={}\nstdout_truncated={}\nstderr_truncated={}\n\n[stdout]\n{}\n\n[stderr]\n{}\n",
        kind.label(),
        result.label(),
        if operation_applied { 1 } else { 0 },
        script_summary(script),
        if timed_out { 1 } else { 0 },
        if stdout_truncated { 1 } else { 0 },
        if stderr_truncated { 1 } else { 0 },
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    atomic_write(&root.join("last.log"), log.as_bytes(), 0o600)?;
    let command_log = format!(
        "time={timestamp}\naction={action}\noperation_kind={}\noperation_result={}\nroute={route}\nexit={code}\nupstream_status={upstream_status}\npost_check={post_check}\nstate_persist={state_persist}\noperation_applied={}\n{operation_state_field}audit_status=ok\ncommand={command}\nrunner_command={command}\n{commands}script_summary={}\ntimed_out={}\nstdout_truncated={}\nstderr_truncated={}\n",
        kind.label(),
        result.label(),
        if operation_applied { 1 } else { 0 },
        script_summary(script),
        if timed_out { 1 } else { 0 },
        if stdout_truncated { 1 } else { 0 },
        if stderr_truncated { 1 } else { 0 },
    );
    atomic_write(
        &root.join("last-command.log"),
        command_log.as_bytes(),
        0o600,
    )?;
    if let Some(status) = status {
        // Publish mutation status last so readers never observe it referring
        // to command/log files that were not fully replaced. Queries retain
        // their audit logs without replacing the last mutation summary.
        atomic_write(&root.join("last.status"), status.as_bytes(), 0o600)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn write_test_audit(root: &std::path::Path, kind: OperationKind) -> Result<(), String> {
    let output = Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let state = (kind == OperationKind::Mutation).then_some(OperationState::Applied);
    write_operation_log_at(
        &AuditRecord {
            action: if kind == OperationKind::Query {
                "extra-diagnose-audio"
            } else {
                "apply"
            },
            kind,
            result: OperationResult::Success,
            route: "test",
            code: 0,
            command: "/system/bin/sh -s",
            business_commands: &[],
            script: "",
            output: &output,
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
            upstream_status: "ok",
            post_check: "not-required",
            state_persist: "ok",
            operation_applied: kind == OperationKind::Mutation,
            operation_state: state,
        },
        root,
    )
}

fn render_business_commands(commands: &[String]) -> String {
    let mut rendered = String::new();
    for (index, command) in commands.iter().enumerate() {
        rendered.push_str(&format!("business_command_{}={}\n", index + 1, command));
    }
    rendered
}
