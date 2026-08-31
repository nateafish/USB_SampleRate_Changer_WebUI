use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::domain::NamespaceInfo;
use crate::process::execute_output;

const ANDROID_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const PID_QUERY_TIMEOUT: Duration = Duration::from_secs(1);
const DUMPSYS_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const SMALL_QUERY_OUTPUT_LIMIT: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum A2dpState {
    Connected,
    Disconnected,
    Unknown,
}

impl A2dpState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn bluetooth_a2dp_state() -> A2dpState {
    let Some(stdout) = command_stdout(
        "dumpsys",
        &["audio"],
        ANDROID_QUERY_TIMEOUT,
        DUMPSYS_OUTPUT_LIMIT,
    ) else {
        return A2dpState::Unknown;
    };
    if bluetooth_a2dp_connected_in_dump(&String::from_utf8_lossy(&stdout)) {
        A2dpState::Connected
    } else {
        A2dpState::Disconnected
    }
}

pub(crate) fn bluetooth_a2dp_connected_in_dump(dump: &str) -> bool {
    let Some((_, connected_and_rest)) = dump.split_once("Connected devices:") else {
        return false;
    };
    let connected = connected_and_rest
        .split_once("APM Connected device")
        .map(|(section, _)| section)
        .unwrap_or(connected_and_rest);
    if !connected.contains("(bt_a2dp)") {
        return false;
    }

    let Some((_, music_and_rest)) = dump.split_once("- STREAM_MUSIC:") else {
        return false;
    };
    let music = music_and_rest
        .split_once("\n- STREAM_")
        .map(|(section, _)| section)
        .unwrap_or(music_and_rest);
    music
        .lines()
        .any(|line| line.trim_start().starts_with("Devices:") && line.contains("bt_a2dp"))
}

fn wait_for_bluetooth_a2dp(attempts: usize, delay: Duration) -> A2dpState {
    let mut last_known = A2dpState::Unknown;
    for _ in 0..attempts {
        match bluetooth_a2dp_state() {
            A2dpState::Connected => return A2dpState::Connected,
            A2dpState::Disconnected => last_known = A2dpState::Disconnected,
            A2dpState::Unknown => {}
        }
        std::thread::sleep(delay);
    }
    last_known
}

pub(crate) fn verify_a2dp_route() -> Result<&'static str, String> {
    // AudioService can retain the old device list briefly after audioserver has
    // restarted. Let the new policy instance settle before deciding that A2DP
    // recovered without intervention.
    std::thread::sleep(Duration::from_secs(3));
    match wait_for_bluetooth_a2dp(8, Duration::from_millis(500)) {
        A2dpState::Connected => Ok("verified"),
        A2dpState::Unknown => Err(
            "audio policy was applied, but A2DP route verification was unavailable".to_string(),
        ),
        A2dpState::Disconnected => Err(
            "audio policy was applied, but the connected headset must be explicitly disconnected and reconnected before STREAM_MUSIC returns to A2DP"
                .to_string(),
        ),
    }
}

pub(crate) fn namespace_info() -> NamespaceInfo {
    let audio_pid = audioserver_pid();
    NamespaceInfo {
        self_ns: read_namespace_link(Path::new("/proc/self/ns/mnt")),
        audio_ns: audio_pid
            .and_then(|pid| read_namespace_link(Path::new(&format!("/proc/{pid}/ns/mnt")))),
        audio_pid,
    }
}

fn read_namespace_link(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|value| value.to_string_lossy().into_owned())
}

fn audioserver_pid() -> Option<u32> {
    let from_pidof = command_stdout(
        "pidof",
        &["audioserver"],
        PID_QUERY_TIMEOUT,
        SMALL_QUERY_OUTPUT_LIMIT,
    )
    .and_then(|stdout| {
        String::from_utf8_lossy(&stdout)
            .split_whitespace()
            .next()
            .and_then(|pid| pid.parse().ok())
    });
    if from_pidof.is_some() {
        return from_pidof;
    }
    command_stdout(
        "getprop",
        &["init.svc_debug_pid.audioserver"],
        PID_QUERY_TIMEOUT,
        SMALL_QUERY_OUTPUT_LIMIT,
    )
    .and_then(|stdout| String::from_utf8_lossy(&stdout).trim().parse().ok())
}

fn command_stdout(
    program: &str,
    args: &[&str],
    timeout: Duration,
    output_limit: usize,
) -> Option<Vec<u8>> {
    let mut command = Command::new(program);
    command.args(args);
    let result = execute_output(&mut command, timeout, output_limit).ok()?;
    if result.timed_out || result.stdout_truncated || !result.output.status.success() {
        return None;
    }
    Some(result.output.stdout)
}
