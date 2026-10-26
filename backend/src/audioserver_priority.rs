use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::operation::{acquire_operation_lock_at, OperationLock};
use crate::paths::{atomic_write, ensure_state_layout, log_root, state_root};

pub(crate) const TARGET_NICE: i32 = -10;

const ENABLED_FILE: &str = "audioserver-priority.enabled";
const BASELINE_FILE: &str = "audioserver-priority.baseline";
const RECONCILE_LOCK: &str = "audioserver-priority.lock";
const WATCH_LOCK: &str = "audioserver-priority-watch.lock";
const WATCH_PID: &str = "audioserver-priority-watch.pid";
const WATCH_LOG: &str = "audioserver-priority.log";
const WATCH_ARGUMENT: &str = "_audioserver-priority-watch";
const LEGACY_WATCH_DIR: &str = "audioserver-priority-watch";
const LEGACY_STOP_FILE: &str = "audioserver-priority.stop";

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PriorityChange {
    pub(crate) adjusted: usize,
    pub(crate) restored: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct Baseline {
    boot_id: String,
    pid: u32,
    tasks: BTreeMap<u32, i32>,
}

#[derive(Debug, PartialEq, Eq)]
struct TaskStat {
    nice: i32,
    policy: u32,
}

pub(crate) fn enabled() -> bool {
    fs::read_to_string(state_root().join(ENABLED_FILE)).is_ok_and(|value| value.trim() == "1")
}

pub(crate) fn set_enabled(value: bool) -> Result<PriorityChange, String> {
    ensure_state_layout()?;
    let _reconcile_lock = acquire_reconcile_lock()?;
    if value {
        atomic_write(&state_root().join(ENABLED_FILE), b"1\n", 0o600)?;
        match reconcile_enabled() {
            Ok(change) => Ok(change),
            Err(error) => {
                remove_if_present(&state_root().join(ENABLED_FILE))?;
                let _ = restore_tasks();
                Err(error)
            }
        }
    } else {
        remove_if_present(&state_root().join(ENABLED_FILE))?;
        restore_tasks()
    }
}

pub(crate) fn start_watch() -> Result<(), String> {
    ensure_state_layout()?;
    stop_legacy_watch()?;
    remove_if_present(&log_root().join(LEGACY_STOP_FILE))?;
    if !enabled() && !state_root().join(BASELINE_FILE).is_file() {
        return Ok(());
    }
    let pid_path = log_root().join(WATCH_PID);
    if let Some(pid) = read_pid(&pid_path) {
        if is_watch_process(pid) {
            return Ok(());
        }
        remove_if_present(&pid_path)?;
    }

    let log_path = log_root().join(WATCH_LOG);
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .map_err(|error| format!("cannot open {}: {error}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("cannot clone priority monitor log: {error}"))?;
    Command::new(
        std::env::current_exe()
            .map_err(|error| format!("cannot resolve controller executable: {error}"))?,
    )
    .arg(WATCH_ARGUMENT)
    .stdin(Stdio::null())
    .stdout(Stdio::from(stdout))
    .stderr(Stdio::from(stderr))
    .spawn()
    .map_err(|error| format!("cannot start audioserver priority monitor: {error}"))?;

    for _ in 0..20 {
        if read_pid(&pid_path).is_some_and(is_watch_process) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("audioserver priority monitor did not start".to_string())
}

pub(crate) fn stop_watch() -> Result<(), String> {
    let pid_path = log_root().join(WATCH_PID);
    if let Some(pid) = read_pid(&pid_path) {
        stop_process(pid, is_watch_process)?;
    }
    remove_if_present(&pid_path)?;
    stop_legacy_watch()
}

pub(crate) fn watch_loop() -> Result<(), String> {
    ensure_state_layout()?;
    let _watch_lock = acquire_operation_lock_at(&log_root().join(WATCH_LOCK))?;
    let pid_path = log_root().join(WATCH_PID);
    atomic_write(
        &pid_path,
        format!("{}\n", std::process::id()).as_bytes(),
        0o600,
    )?;
    let _pid_cleanup = RemoveOnDrop(pid_path);

    loop {
        if !std::env::current_exe().is_ok_and(|path| path.is_file()) {
            return Ok(());
        }
        let baseline_exists = state_root().join(BASELINE_FILE).is_file();
        if enabled() || baseline_exists {
            if let Err(error) = reconcile() {
                eprintln!("audioserver priority reconcile failed: {error}");
            }
        }
        thread::sleep(if enabled() {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(5)
        });
    }
}

fn reconcile() -> Result<PriorityChange, String> {
    let _reconcile_lock = acquire_reconcile_lock()?;
    if enabled() {
        reconcile_enabled()
    } else {
        restore_tasks()
    }
}

fn acquire_reconcile_lock() -> Result<OperationLock, String> {
    let path = log_root().join(RECONCILE_LOCK);
    for _ in 0..50 {
        match acquire_operation_lock_at(&path) {
            Ok(lock) => return Ok(lock),
            Err(error) if error == "another audio operation is still running" => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(format!("cannot lock audioserver priority state: {error}"));
            }
        }
    }
    Err("timed out waiting for audioserver priority state lock".to_string())
}

fn reconcile_enabled() -> Result<PriorityChange, String> {
    let Some(pid) = audioserver_pid()? else {
        return Ok(PriorityChange::default());
    };
    let boot_id = boot_id()?;
    let path = state_root().join(BASELINE_FILE);
    let mut baseline = read_baseline(&path)
        .filter(|baseline| baseline.boot_id == boot_id && baseline.pid == pid)
        .unwrap_or(Baseline {
            boot_id,
            pid,
            tasks: BTreeMap::new(),
        });
    let mut changed = false;
    for tid in task_ids(pid)? {
        if baseline.tasks.contains_key(&tid) {
            continue;
        }
        if let Some(stat) = task_stat(pid, tid)? {
            baseline.tasks.insert(tid, stat.nice);
            changed = true;
        }
    }
    if changed || !path.is_file() {
        write_baseline(&path, &baseline)?;
    }

    let mut adjusted = 0;
    let mut failures = Vec::new();
    for tid in task_ids(pid)? {
        let Some(stat) = task_stat(pid, tid)? else {
            continue;
        };
        if stat.nice <= TARGET_NICE || matches!(stat.policy, 1 | 2 | 6) {
            continue;
        }
        if let Err(error) = set_task_nice(tid, TARGET_NICE) {
            if task_path(pid, tid).is_dir() {
                failures.push(format!("{tid}: {error}"));
            }
        } else {
            adjusted += 1;
        }
    }
    if failures.is_empty() {
        Ok(PriorityChange {
            adjusted,
            restored: 0,
        })
    } else {
        Err(format!(
            "cannot raise audioserver thread priority: {}",
            failures.join(", ")
        ))
    }
}

fn restore_tasks() -> Result<PriorityChange, String> {
    let path = state_root().join(BASELINE_FILE);
    let Some(baseline) = read_baseline(&path) else {
        remove_if_present(&path)?;
        return Ok(PriorityChange::default());
    };
    let current_boot = boot_id()?;
    let current_pid = audioserver_pid()?;
    if baseline.boot_id != current_boot || current_pid != Some(baseline.pid) {
        remove_if_present(&path)?;
        return Ok(PriorityChange::default());
    }

    let mut restored = 0;
    let mut failures = Vec::new();
    for (tid, original_nice) in baseline.tasks {
        let Some(stat) = task_stat(baseline.pid, tid)? else {
            continue;
        };
        if stat.nice != TARGET_NICE {
            continue;
        }
        if let Err(error) = set_task_nice(tid, original_nice) {
            if task_path(baseline.pid, tid).is_dir() {
                failures.push(format!("{tid}: {error}"));
            }
        } else {
            restored += 1;
        }
    }
    if failures.is_empty() {
        remove_if_present(&path)?;
        Ok(PriorityChange {
            adjusted: 0,
            restored,
        })
    } else {
        Err(format!(
            "cannot restore audioserver thread priority: {}",
            failures.join(", ")
        ))
    }
}

fn boot_id() -> Result<String, String> {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("cannot read boot ID: {error}"))
}

fn audioserver_pid() -> Result<Option<u32>, String> {
    let entries = fs::read_dir("/proc").map_err(|error| format!("cannot scan /proc: {error}"))?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if fs::read_to_string(entry.path().join("comm"))
            .is_ok_and(|name| name.trim() == "audioserver")
        {
            return Ok(Some(pid));
        }
    }
    Ok(None)
}

fn task_ids(pid: u32) -> Result<Vec<u32>, String> {
    let path = format!("/proc/{pid}/task");
    let entries = fs::read_dir(&path).map_err(|error| format!("cannot scan {path}: {error}"))?;
    let mut ids: Vec<u32> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

fn task_path(pid: u32, tid: u32) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/proc/{pid}/task/{tid}"))
}

fn task_stat(pid: u32, tid: u32) -> Result<Option<TaskStat>, String> {
    let path = task_path(pid, tid).join("stat");
    match fs::read_to_string(&path) {
        Ok(stat) => parse_task_stat(&stat)
            .map(Some)
            .ok_or_else(|| format!("invalid task stat: {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn parse_task_stat(stat: &str) -> Option<TaskStat> {
    let fields: Vec<&str> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some(TaskStat {
        nice: fields.get(16)?.parse().ok()?,
        policy: fields.get(38)?.parse().ok()?,
    })
}

fn set_task_nice(tid: u32, nice: i32) -> Result<(), std::io::Error> {
    let result = unsafe { setpriority(0, tid, nice) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn read_baseline(path: &std::path::Path) -> Option<Baseline> {
    let content = fs::read_to_string(path).ok()?;
    let mut boot_id = None;
    let mut pid = None;
    let mut tasks = BTreeMap::new();
    for line in content.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "boot_id" => boot_id = Some(value.to_string()),
            "pid" => pid = value.parse().ok(),
            _ => {
                if let Some(tid) = key
                    .strip_prefix("task_")
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    tasks.insert(tid, value.parse().ok()?);
                }
            }
        }
    }
    Some(Baseline {
        boot_id: boot_id?,
        pid: pid?,
        tasks,
    })
}

fn write_baseline(path: &std::path::Path, baseline: &Baseline) -> Result<(), String> {
    let mut content = format!("boot_id={}\npid={}\n", baseline.boot_id, baseline.pid);
    for (tid, nice) in &baseline.tasks {
        content.push_str(&format!("task_{tid}={nice}\n"));
    }
    atomic_write(path, content.as_bytes(), 0o600)
}

fn read_pid(path: &std::path::Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn is_watch_process(pid: u32) -> bool {
    fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|cmdline| {
        cmdline
            .split(|byte| *byte == 0)
            .any(|argument| argument == WATCH_ARGUMENT.as_bytes())
    })
}

fn stop_legacy_watch() -> Result<(), String> {
    let directory = log_root().join(LEGACY_WATCH_DIR);
    let pid_path = directory.join("pid");
    if let Some(pid) = read_pid(&pid_path) {
        stop_process(pid, is_legacy_watch_process)?;
    }
    remove_if_present(&pid_path)?;
    match fs::remove_dir(&directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", directory.display())),
    }
}

fn is_legacy_watch_process(pid: u32) -> bool {
    fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|cmdline| {
        let arguments: Vec<&[u8]> = cmdline.split(|byte| *byte == 0).collect();
        arguments
            .iter()
            .any(|argument| argument.ends_with(b"audioserver-priority.sh"))
            && arguments.iter().any(|argument| *argument == b"watch")
    })
}

fn stop_process(pid: u32, matches_process: fn(u32) -> bool) -> Result<(), String> {
    if !matches_process(pid) {
        return Ok(());
    }
    let result = unsafe { kill(pid as i32, 15) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != ErrorKind::NotFound {
            return Err(format!("cannot stop audioserver priority monitor: {error}"));
        }
    }
    for _ in 0..20 {
        if !matches_process(pid) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    if matches_process(pid) {
        Err("audioserver priority monitor did not stop".to_string())
    } else {
        Ok(())
    }
}

fn remove_if_present(path: &std::path::Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

unsafe extern "C" {
    fn setpriority(which: i32, who: u32, priority: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nice_after_a_task_name_with_spaces_and_parentheses() {
        let fields: Vec<String> = (1..=37).map(|value| value.to_string()).collect();
        let mut stat = format!("123 (Audio Worker (A)) S {}", fields.join(" "));
        stat.push_str(" 2");
        let parsed = parse_task_stat(&stat).unwrap();
        assert_eq!(parsed.nice, 16);
        assert_eq!(parsed.policy, 2);
    }

    #[test]
    fn parses_the_shell_compatible_baseline_format() {
        let root =
            std::env::temp_dir().join(format!("usbsr-priority-baseline-{}", std::process::id()));
        let baseline = Baseline {
            boot_id: "boot".to_string(),
            pid: 42,
            tasks: BTreeMap::from([(42, 0), (43, -16)]),
        };
        write_baseline(&root, &baseline).unwrap();
        assert_eq!(read_baseline(&root), Some(baseline));
        fs::remove_file(root).unwrap();
    }
}
