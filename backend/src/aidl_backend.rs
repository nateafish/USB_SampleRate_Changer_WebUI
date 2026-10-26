//! Runtime adapter for Audio Core AIDL policies.
//!
//! `aidl.rs` contains the dependency-free topology and module-XML model.  This
//! file is the small Android-facing boundary around that model: it discovers
//! the live policy, finds a matching QTI module XML source, and (only after all
//! checks pass) performs the optional bind-mount/reload operation.  The
//! traditional XML/HIDL path never enters this module.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use crate::aidl::{self, AudioModule, AudioPolicySnapshot, PolicyPlan};
use crate::android::{bluetooth_a2dp_state, namespace_info, A2dpState};
use crate::catalog::{DOCUMENTED_RATES, POLICIES};
use crate::cli::normalize_sample_rate;
use crate::domain::{Action, Settings};
use crate::operation::{
    acquire_operation_lock, execute_script_locked, finalize_operation, script_runner,
    OperationContext, OperationKind, OperationLock, Persistence,
};
use crate::paths::{atomic_write, ensure_state_layout, state_root};
use crate::scripts::{shell_quote, validate_settings_for_action};
use crate::state::bool_number;
use crate::state::StateStore;

const AIDL_TIMEOUT: Duration = Duration::from_secs(60);
const AIDL_RESTART_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub(crate) struct AidlConfigSource {
    pub(crate) path: PathBuf,
    pub(crate) xml: String,
    pub(crate) model: AudioModule,
    pub(crate) matches: usize,
    pub(crate) total: usize,
}

/// Return true only when the running framework explicitly reports an AIDL
/// AudioPolicy.  A failed read is treated as false so legacy devices retain
/// their original path and error handling.
pub(crate) fn active_is_aidl() -> bool {
    audio_policy_snapshot().is_ok_and(|snapshot| snapshot.is_aidl())
}

pub(crate) fn audio_policy_snapshot() -> Result<AudioPolicySnapshot, String> {
    let mut command = Command::new("dumpsys");
    command.arg("media.audio_policy");
    let result =
        crate::process::execute_output(&mut command, Duration::from_secs(5), 4 * 1024 * 1024)
            .map_err(|error| format!("cannot run media.audio_policy dump: {error}"))?;
    if result.timed_out || result.stdout_truncated {
        return Err("media.audio_policy dump was incomplete".to_string());
    }
    let output = result.output;
    if !output.status.success() {
        return Err(format!(
            "media.audio_policy dump failed with status {}",
            output.status
        ));
    }
    Ok(aidl::parse_audio_policy_dump(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Find the one module XML whose complete port/route topology matches the
/// framework's primary/default module.  Merely finding a file is not enough:
/// vendor and ODM images often contain several similarly named XML files.
pub(crate) fn discover_aidl_primary_config(
    snapshot: &AudioPolicySnapshot,
) -> Result<AidlConfigSource, String> {
    if !snapshot.is_aidl() {
        return Err(format!(
            "active audio policy is not AIDL: {}",
            snapshot.config_source
        ));
    }
    let runtime = snapshot
        .modules
        .iter()
        .find(|module| module.name == "primary" || module.name == "default")
        .ok_or_else(|| "runtime primary/default module was not found".to_string())?;

    let mut candidates = BTreeSet::new();
    for root in [
        "/odm/etc/audio",
        "/vendor/etc/audio",
        "/product/etc/audio",
        "/system_ext/etc/audio",
        "/system/etc/audio",
    ] {
        collect_module_xml_files(Path::new(root), 4, &mut candidates);
    }

    let mut matches = Vec::new();
    for path in candidates {
        let Ok(xml) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(model) = aidl::parse_module_xml(&xml) else {
            continue;
        };
        let (matched, total) = aidl::topology_match_score(runtime, &model);
        if total > 0 && matched == total {
            matches.push(AidlConfigSource {
                path,
                xml,
                model,
                matches: matched,
                total,
            });
        }
    }

    // An exact primary filename is preferred only when it is unique.  If two
    // overlays expose the same topology, refusing to guess is safer than
    // stacking a bind mount over an unrelated module's source.
    if matches.len() > 1 {
        let exact = matches
            .iter()
            .filter(|source| {
                source.path.file_name() == Some(OsStr::new("audio_module_config_primary.xml"))
            })
            .count();
        if exact == 1 {
            return Ok(matches
                .into_iter()
                .find(|source| {
                    source.path.file_name() == Some(OsStr::new("audio_module_config_primary.xml"))
                })
                .expect("the unique exact source exists"));
        }
    }
    match matches.len() {
        0 => Err("no module XML fully matches the live AIDL topology".to_string()),
        1 => Ok(matches.remove(0)),
        count => Err(format!(
            "{count} module XML files fully match the live AIDL topology; refusing an ambiguous writer"
        )),
    }
}

fn collect_module_xml_files(root: &Path, depth: usize, output: &mut BTreeSet<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "audio_module_config_primary.xml"
                || (name.starts_with("audio_module_config_") && name.ends_with(".xml"))
            {
                output.insert(path);
            }
        } else if file_type.is_dir() {
            collect_module_xml_files(&path, depth - 1, output);
        }
    }
}

pub(crate) fn discover_audio_core_aidl_versions() -> Vec<u32> {
    let mut manifests = BTreeSet::new();
    for root in [
        "/vendor/etc/vintf",
        "/odm/etc/vintf",
        "/product/etc/vintf",
        "/system_ext/etc/vintf",
        "/system/etc/vintf",
    ] {
        let root = Path::new(root);
        let manifest = root.join("manifest.xml");
        if manifest.is_file() {
            manifests.insert(manifest);
        }
        collect_xml_files(&root.join("manifest"), 5, &mut manifests);
    }
    let mut versions = manifests
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .flat_map(|xml| aidl::parse_audio_core_aidl_versions(&xml))
        .collect::<Vec<_>>();
    versions.sort_unstable();
    versions.dedup();
    versions
}

fn collect_xml_files(root: &Path, depth: usize, output: &mut BTreeSet<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && path.extension() == Some(OsStr::new("xml")) {
            output.insert(path);
        } else if file_type.is_dir() {
            collect_xml_files(&path, depth - 1, output);
        }
    }
}

pub(crate) fn supported_audio_core_aidl_version(versions: &[u32]) -> Option<u32> {
    match versions {
        [version @ (2 | 3)] => Some(*version),
        _ => None,
    }
}

fn discover_audio_hal_service() -> String {
    let Ok(output) = Command::new("getprop").output() else {
        return "vendor.audio-hal-aidl".to_string();
    };
    let mut fallback = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((raw_key, raw_state)) = line.split_once("]: [") else {
            continue;
        };
        let key = raw_key.trim_start_matches('[');
        let state = raw_state.trim_end_matches(']');
        let Some(name) = key.strip_prefix("init.svc.") else {
            continue;
        };
        let lowered = name.to_ascii_lowercase();
        if !lowered.contains("audio") || !lowered.contains("hal") {
            continue;
        }
        if lowered.contains("aidl") && state == "running" {
            return name.to_string();
        }
        if fallback.is_none() && (state == "running" || state == "stopped") {
            fallback = Some(name.to_string());
        }
    }
    fallback.unwrap_or_else(|| "vendor.audio-hal-aidl".to_string())
}

pub(crate) fn render_status() -> String {
    let mut output = String::new();
    match audio_policy_snapshot() {
        Ok(snapshot) if snapshot.is_aidl() => {
            push_line(&mut output, "audio_backend", "aidl");
            push_line(&mut output, "audio_config_source", &snapshot.config_source);
            let versions = discover_audio_core_aidl_versions();
            push_line(
                &mut output,
                "aidl_core_versions",
                &versions
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            if versions.len() == 1 {
                push_line(&mut output, "aidl_core_version", &versions[0].to_string());
            }
            push_line(
                &mut output,
                "aidl_version_supported",
                &bool_number(supported_audio_core_aidl_version(&versions).is_some()).to_string(),
            );
            push_line(
                &mut output,
                "aidl_hal_service",
                &discover_audio_hal_service(),
            );
            push_line(
                &mut output,
                "aidl_module_count",
                &snapshot.modules.len().to_string(),
            );
            push_line(
                &mut output,
                "aidl_usb_output_device_count",
                &snapshot.usb_output_devices().len().to_string(),
            );
            let ports = snapshot.usb_reachable_mix_ports();
            push_line(
                &mut output,
                "aidl_usb_reachable_mix_port_count",
                &ports.len().to_string(),
            );
            let behaviors = ports
                .iter()
                .map(|(_, port)| port.behavior())
                .collect::<BTreeSet<_>>();
            push_line(
                &mut output,
                "aidl_usb_port_behaviors",
                &behaviors.into_iter().collect::<Vec<_>>().join(","),
            );
            push_line(
                &mut output,
                "aidl_usb_port_summary",
                &ports
                    .iter()
                    .map(|(module, port)| format!("{module}:{}:{}", port.name, port.behavior()))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            let dynamic_count = ports
                .iter()
                .filter(|(_, port)| {
                    port.profiles.is_empty()
                        || port.profiles.iter().any(|profile| {
                            profile.dynamic_rate
                                || profile.dynamic_format
                                || profile.dynamic_channels
                        })
                })
                .count();
            push_line(
                &mut output,
                "aidl_usb_dynamic_port_count",
                &dynamic_count.to_string(),
            );
            for (policy, _, _) in POLICIES.iter().filter(|(policy, _, _)| *policy != "legacy") {
                let rates = DOCUMENTED_RATES
                    .iter()
                    .map(|(rate, _)| *rate)
                    .filter(|rate| aidl::plan_policy(&snapshot, policy, *rate, "32").is_ok())
                    .map(|rate| rate.to_string())
                    .collect::<Vec<_>>();
                push_line(
                    &mut output,
                    &format!("aidl_supported_rates_{}", policy.replace('-', "_")),
                    &rates.join(","),
                );
            }
            match discover_aidl_primary_config(&snapshot) {
                Ok(source) => {
                    push_line(&mut output, "aidl_config_adapter", "module-xml");
                    push_line(
                        &mut output,
                        "aidl_config_path",
                        &source.path.display().to_string(),
                    );
                    push_line(
                        &mut output,
                        "aidl_config_topology",
                        &format!("{}/{}", source.matches, source.total),
                    );
                    push_line(&mut output, "aidl_preview_supported", "1");
                    push_line(
                        &mut output,
                        "aidl_apply_supported",
                        &bool_number(supported_audio_core_aidl_version(&versions).is_some())
                            .to_string(),
                    );
                    push_line(
                        &mut output,
                        "aidl_mount_active",
                        &bool_number(path_is_mounted(&source.path)).to_string(),
                    );
                }
                Err(error) => {
                    push_line(&mut output, "aidl_config_adapter", "read-only");
                    push_line(&mut output, "aidl_preview_supported", "0");
                    push_line(&mut output, "aidl_apply_supported", "0");
                    push_line(&mut output, "aidl_config_error", &sanitize(&error));
                }
            }
        }
        Ok(snapshot) => {
            push_line(&mut output, "audio_backend", "traditional");
            push_line(&mut output, "audio_config_source", &snapshot.config_source);
        }
        Err(error) => {
            push_line(&mut output, "audio_backend", "unknown");
            push_line(&mut output, "audio_backend_error", &sanitize(&error));
        }
    }
    output
}

fn push_line(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push('=');
    output.push_str(&sanitize(value));
    output.push('\n');
}

fn sanitize(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

pub(crate) fn print_inspect() -> Result<(), String> {
    let snapshot = audio_policy_snapshot()?;
    print_snapshot(&snapshot)
}

pub(crate) fn print_inspect_stdin() -> Result<(), String> {
    let mut dump = String::new();
    std::io::stdin()
        .read_to_string(&mut dump)
        .map_err(|error| format!("cannot read audio policy dump: {error}"))?;
    let snapshot = aidl::parse_audio_policy_dump(&dump);
    print_snapshot(&snapshot)
}

fn print_snapshot(snapshot: &AudioPolicySnapshot) -> Result<(), String> {
    if !snapshot.is_aidl() {
        return Err(format!(
            "active audio policy is not AIDL: {}",
            snapshot.config_source
        ));
    }
    println!("config_source={}", snapshot.config_source);
    for module in &snapshot.modules {
        println!("module={}", module.name);
        for port in &module.output_mix_ports {
            let dynamic = port.profiles.iter().any(|profile| {
                profile.dynamic_rate || profile.dynamic_format || profile.dynamic_channels
            });
            println!(
                "mix_port={}|{}|{}|{}|{}",
                module.name,
                port.name,
                port.behavior(),
                if dynamic { "dynamic" } else { "static" },
                port.unique_rates()
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        for port in &module.device_ports {
            println!(
                "device_port={}|{}|{}",
                module.name,
                port.name,
                port.device_type.as_deref().unwrap_or("")
            );
        }
        for route in &module.routes {
            println!(
                "route={}|{}|{}",
                module.name,
                route.sink,
                route.sources.join(",")
            );
        }
    }
    for (module, port) in snapshot.usb_reachable_mix_ports() {
        println!(
            "usb_reachable_mix_port={module}|{}|{}",
            port.name,
            port.behavior()
        );
    }
    Ok(())
}

pub(crate) fn print_preview(policy: &str, rate: &str, bit_depth: &str) -> Result<(), String> {
    let snapshot = audio_policy_snapshot()?;
    let source = discover_aidl_primary_config(&snapshot)?;
    let rate = normalize_sample_rate(rate)?;
    let runtime = primary_runtime(&snapshot)?;
    let plan = aidl::plan_policy(&snapshot, policy, rate, bit_depth)?;
    let candidate = aidl::apply_plan_to_module_xml(&source.xml, &source.model.name, &plan)?;
    let after = aidl::parse_module_xml(&candidate)?;
    let (candidate_matches, candidate_total) = aidl::topology_match_score(runtime, &after);
    validate_candidate(
        runtime,
        &source.model,
        &after,
        candidate_matches,
        candidate_total,
    )?;
    println!("audio_backend=aidl");
    println!("config_adapter=module-xml");
    println!("config_path={}", source.path.display());
    println!("policy={policy}");
    println!("runtime_module={}", runtime.name);
    println!("config_module={}", source.model.name);
    println!("source_topology={}/{}", source.matches, source.total);
    println!("candidate_topology={candidate_matches}/{candidate_total}");
    for target in &plan.targets {
        println!(
            "target={}|{}|{}|{}|{}",
            target.module, target.port, target.behavior, target.rate, target.pcm_type
        );
    }
    for (port, behavior, reason) in &plan.skipped {
        println!("skipped={port}|{behavior}|{reason}");
    }
    for change in aidl::profile_diff(&source.model, &after) {
        println!("profile_change={change}");
    }
    println!("structure_changes=0");
    println!("candidate_bytes={}", candidate.len());
    println!("apply_enabled=0");
    Ok(())
}

pub(crate) fn print_match_stdin(path: &str) -> Result<(), String> {
    let dump = read_stdin_text("audio policy dump")?;
    let xml = fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let snapshot = aidl::parse_audio_policy_dump(&dump);
    let runtime = primary_runtime(&snapshot)?;
    let config = aidl::parse_module_xml(&xml)?;
    let (matched, total) = aidl::topology_match_score(runtime, &config);
    println!("runtime_module={}", runtime.name);
    println!("config_module={}", config.name);
    println!("topology_matches={matched}");
    println!("topology_total={total}");
    println!(
        "topology_match_percent={}",
        if total == 0 { 0 } else { matched * 100 / total }
    );
    Ok(())
}

pub(crate) fn print_rewrite_validate_stdin(
    path: &str,
    port: &str,
    rate: &str,
    bit_depth: &str,
) -> Result<(), String> {
    let dump = read_stdin_text("audio policy dump")?;
    let source =
        fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let rate = normalize_sample_rate(rate)?;
    let pcm_type = aidl::pcm_type_for_bit_depth(bit_depth)?;
    let snapshot = aidl::parse_audio_policy_dump(&dump);
    let runtime = primary_runtime(&snapshot)?;
    let source_model = aidl::parse_module_xml(&source)?;
    let (source_matches, source_total) = aidl::topology_match_score(runtime, &source_model);
    if source_total == 0 || source_matches != source_total {
        return Err(format!(
            "source XML topology mismatch: {source_matches}/{source_total}"
        ));
    }
    let rewritten = aidl::rewrite_pcm_mix_port(&source, port, rate, pcm_type)?;
    let rewritten_model = aidl::parse_module_xml(&rewritten)?;
    let (result_matches, result_total) = aidl::topology_match_score(runtime, &rewritten_model);
    if result_matches != result_total {
        return Err(format!(
            "rewritten XML topology mismatch: {result_matches}/{result_total}"
        ));
    }
    let target = rewritten_model
        .output_mix_ports
        .iter()
        .find(|candidate| candidate.name == port)
        .ok_or_else(|| "rewritten mix port disappeared".to_string())?;
    println!("source_topology={source_matches}/{source_total}");
    println!("result_topology={result_matches}/{result_total}");
    println!("target_mix_port={}", target.name);
    println!("target_behavior={}", target.behavior());
    println!("target_rate={rate}");
    println!("target_pcm_type={pcm_type}");
    println!("source_bytes={}", source.len());
    println!("result_bytes={}", rewritten.len());
    Ok(())
}

pub(crate) fn print_plan_stdin(
    path: &str,
    policy: &str,
    rate: &str,
    bit_depth: &str,
    output_path: Option<&str>,
) -> Result<(), String> {
    let dump = read_stdin_text("audio policy dump")?;
    let rate = normalize_sample_rate(rate)?;
    let source =
        fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let snapshot = aidl::parse_audio_policy_dump(&dump);
    let runtime = primary_runtime(&snapshot)?;
    let before = aidl::parse_module_xml(&source)?;
    let (source_matches, source_total) = aidl::topology_match_score(runtime, &before);
    if source_total == 0 || source_matches != source_total {
        return Err(format!(
            "source XML topology mismatch: {source_matches}/{source_total}"
        ));
    }
    let plan = aidl::plan_policy(&snapshot, policy, rate, bit_depth)?;
    let candidate = aidl::apply_plan_to_module_xml(&source, &before.name, &plan)?;
    let after = aidl::parse_module_xml(&candidate)?;
    let (result_matches, result_total) = aidl::topology_match_score(runtime, &after);
    validate_candidate(runtime, &before, &after, result_matches, result_total)?;
    println!("policy={}", plan.policy);
    println!("runtime_module={}", runtime.name);
    println!("config_module={}", before.name);
    println!("source_topology={source_matches}/{source_total}");
    println!("candidate_topology={result_matches}/{result_total}");
    for target in &plan.targets {
        println!(
            "target={}|{}|{}|{}|{}",
            target.module, target.port, target.behavior, target.rate, target.pcm_type
        );
    }
    for (port, behavior, reason) in &plan.skipped {
        println!("skipped={port}|{behavior}|{reason}");
    }
    for change in aidl::profile_diff(&before, &after) {
        println!("profile_change={change}");
    }
    println!("structure_changes=0");
    if let Some(destination) = output_path {
        validate_output_path(destination)?;
        atomic_write(Path::new(destination), candidate.as_bytes(), 0o644)?;
        println!("candidate_written={destination}");
    }
    println!("apply_enabled=0");
    Ok(())
}

fn read_stdin_text(label: &str) -> Result<String, String> {
    let mut content = String::new();
    std::io::stdin()
        .read_to_string(&mut content)
        .map_err(|error| format!("cannot read {label}: {error}"))?;
    Ok(content)
}

fn validate_output_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.contains("..") || path.contains(['\n', '\r']) {
        return Err("candidate output path must be an absolute safe path".to_string());
    }
    Ok(())
}

fn primary_runtime(snapshot: &AudioPolicySnapshot) -> Result<&AudioModule, String> {
    snapshot
        .modules
        .iter()
        .find(|module| module.name == "primary" || module.name == "default")
        .ok_or_else(|| "runtime primary/default module was not found".to_string())
}

fn validate_candidate(
    runtime: &AudioModule,
    before: &AudioModule,
    after: &AudioModule,
    matched: usize,
    total: usize,
) -> Result<(), String> {
    if total == 0 || matched != total {
        return Err(format!(
            "candidate XML topology mismatch: {matched}/{total}"
        ));
    }
    let structural = aidl::structural_diff(before, after);
    if !structural.is_empty() {
        return Err(format!(
            "candidate changes module structure: {}",
            structural.join("; ")
        ));
    }
    // Keep the runtime argument in this helper so future topology gates can
    // compare module identity without changing all call sites.
    let _ = runtime;
    Ok(())
}

fn aidl_state_dir() -> PathBuf {
    state_root().join("aidl")
}

fn aidl_metadata_path() -> PathBuf {
    aidl_state_dir().join("active.conf")
}

fn aidl_original_path() -> PathBuf {
    aidl_state_dir().join("original.xml")
}

fn aidl_candidate_path() -> PathBuf {
    aidl_state_dir().join("current.xml")
}

fn read_aidl_metadata() -> Result<(PathBuf, String), String> {
    let content = fs::read_to_string(aidl_metadata_path())
        .map_err(|error| format!("cannot read AIDL state metadata: {error}"))?;
    let target = content
        .lines()
        .find_map(|line| line.strip_prefix("target="))
        .filter(|value| value.starts_with('/') && !value.contains(".."))
        .map(PathBuf::from)
        .ok_or_else(|| "AIDL state metadata has no safe target".to_string())?;
    let service = content
        .lines()
        .find_map(|line| line.strip_prefix("service="))
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
        .unwrap_or("vendor.audio-hal-aidl")
        .to_string();
    Ok((target, service))
}

fn path_is_mounted(target: &Path) -> bool {
    let Ok(mountinfo) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let target = target.to_string_lossy();
    mountinfo.lines().any(|line| {
        line.split_whitespace()
            .nth(4)
            .is_some_and(|mountpoint| mountpoint == target)
    })
}

fn set_config_context(candidate: &Path, target: &Path) -> Result<(), String> {
    // Android's toybox chcon has no --reference option. Read the target's
    // exact label rather than assuming that ODM and vendor use the same type.
    let reference = Command::new("ls").arg("-Zd").arg(target).output();
    if let Ok(reference) = reference {
        if reference.status.success() {
            if let Some(context) = String::from_utf8_lossy(&reference.stdout)
                .split_whitespace()
                .find(|value| value.starts_with("u:object_r:") && value.contains(":s0"))
            {
                if Command::new("chcon")
                    .arg(context)
                    .arg(candidate)
                    .status()
                    .is_ok_and(|status| status.success())
                {
                    return Ok(());
                }
            }
        }
    }
    // Older toolbox versions do not implement --reference; retain the
    // historical labels as a narrowly scoped fallback.
    for context in [
        "u:object_r:vendor_configs_file:s0",
        "u:object_r:odm_configs_file:s0",
    ] {
        let Ok(status) = Command::new("chcon")
            .args([context])
            .arg(candidate)
            .status()
        else {
            continue;
        };
        if status.success() {
            return Ok(());
        }
    }
    Err(format!(
        "cannot label {} as an audio configuration",
        candidate.display()
    ))
}

fn render_aidl_reload_script(target: &Path, candidate: Option<&Path>, service: &str) -> String {
    let target = shell_quote(&target.to_string_lossy());
    let service = shell_quote(service);
    let mut script = format!(
        "#!/system/bin/sh\nset -u\necho 'controller_upstream_started=1' >&2\ntarget={target}\nservice_name={service}\n"
    );
    script.push_str(
        r#"hal_pid() {
    pid="$(getprop "init.svc_debug_pid.${service_name}" 2>/dev/null)"
    if [ -z "$pid" ]; then pid="$(pidof audiohalservice.qti 2>/dev/null)"; fi
    printf '%s' "$pid"
}
core_service_count() {
    service list 2>/dev/null | grep -c android.hardware.audio.core || true
}
old_hal="$(hal_pid)"
old_audio="$(pidof audioserver 2>/dev/null)"
old_services="$(core_service_count)"
case "$old_services" in ''|*[!0-9]*) old_services=0;; esac
if [ -z "$old_hal" ] || [ -z "$old_audio" ] || [ "$old_services" -lt 1 ]; then
    echo "AIDL audio service baseline is incomplete" >&2
    exit 30
fi
mount_ns="$(readlink /proc/self/ns/mnt)"
if [ -z "$mount_ns" ] || [ "$mount_ns" != "$(readlink /proc/1/ns/mnt)" ] || [ "$mount_ns" != "$(readlink /proc/$old_hal/ns/mnt)" ] || [ "$mount_ns" != "$(readlink /proc/$old_audio/ns/mnt)" ]; then
    echo "AIDL HAL, audioserver and writer must share the global mount namespace" >&2
    exit 30
fi
"#,
    );
    match candidate {
        Some(candidate) => {
            let candidate = shell_quote(&candidate.to_string_lossy());
            script.push_str(&format!(
                "if grep -Fq \" $target \" /proc/self/mountinfo; then umount \"$target\" || exit 31; fi\nmount --bind {candidate} \"$target\" || exit 32\n"
            ));
        }
        None => script.push_str(
            "if grep -Fq \" $target \" /proc/self/mountinfo; then umount \"$target\" || exit 31; fi\n",
        ),
    }
    script.push_str(
        r#"setprop ctl.restart "$service_name"
ok=0
n=0
while [ "$n" -lt 25 ]; do
    new_hal="$(hal_pid)"
    new_audio="$(pidof audioserver 2>/dev/null)"
    services="$(core_service_count)"
    case "$services" in ''|*[!0-9]*) services=0;; esac
    if [ -n "$new_hal" ] && [ -n "$new_audio" ] && [ "$new_hal" != "$old_hal" ] && [ "$new_audio" != "$old_audio" ] && [ "$services" -ge "$old_services" ]; then
        ok=1
        break
    fi
    sleep 1
    n=$((n + 1))
done
if [ "$ok" -ne 1 ]; then
"#,
    );
    if candidate.is_some() {
        script.push_str(
            "    umount \"$target\" 2>/dev/null || true\n    setprop ctl.restart \"$service_name\"\n    sleep 5\n",
        );
    }
    script.push_str(
        r#"    echo "AIDL audio services did not recover" >&2
    exit 33
fi
printf "old_hal=%s\nnew_hal=%s\nold_audioserver=%s\nnew_audioserver=%s\naidl_services_before=%s\naidl_services=%s\n" "$old_hal" "$new_hal" "$old_audio" "$new_audio" "$old_services" "$services"
"#,
    );
    script
}

fn write_metadata(target: &Path, service: &str, source: &[u8]) -> Result<(), String> {
    atomic_write(&aidl_original_path(), source, 0o600)?;
    atomic_write(&aidl_candidate_path(), source, 0o644)?;
    atomic_write(
        &aidl_metadata_path(),
        format!(
            "version=1\ntarget={}\nservice={}\nsource_bytes={}\n",
            target.display(),
            service,
            source.len()
        )
        .as_bytes(),
        0o600,
    )
}

fn remove_metadata() {
    let _ = fs::remove_file(aidl_metadata_path());
    let _ = fs::remove_file(aidl_candidate_path());
    let _ = fs::remove_file(aidl_original_path());
}

pub(crate) fn run_operation(settings: Settings, action: Action) -> Result<i32, String> {
    let module_dir = crate::paths::module_dir()?;
    validate_settings_for_action(&settings, &module_dir, action)?;
    ensure_state_layout()?;
    fs::create_dir_all(aidl_state_dir())
        .map_err(|error| format!("cannot create AIDL state directory: {error}"))?;
    let lock = acquire_operation_lock()?;
    crate::operation::load_state_preflight(&StateStore::new(), matches!(action, Action::Reset))?;
    if matches!(action, Action::Apply) {
        validate_aidl_settings(&settings)?;
    }
    let namespace = namespace_info();
    let (route, command_route) = script_runner(&namespace);
    let a2dp_state = Some(bluetooth_a2dp_state());

    match action {
        Action::Apply => run_apply(
            settings,
            &lock,
            &namespace,
            route,
            &command_route,
            a2dp_state,
        ),
        Action::Reset => run_reset(&lock, &namespace, route, &command_route, a2dp_state),
    }
}

fn validate_aidl_settings(settings: &Settings) -> Result<(), String> {
    if settings.drc
        || settings.force_usbv2
        || settings.force_bluetooth_qti
        || settings.amzm
        || settings.test
        || settings.test_template.is_some()
    {
        return Err(
            "traditional XML template switches are unavailable for AIDL module XML".to_string(),
        );
    }
    Ok(())
}

fn run_apply(
    settings: Settings,
    lock: &OperationLock,
    namespace: &crate::domain::NamespaceInfo,
    route: &'static str,
    command_route: &str,
    a2dp_state: Option<A2dpState>,
) -> Result<i32, String> {
    let versions = discover_audio_core_aidl_versions();
    let version = supported_audio_core_aidl_version(&versions).ok_or_else(|| {
        format!(
            "persistent apply requires exactly one Audio Core AIDL v2 or v3 VINTF declaration (found: {})",
            if versions.is_empty() {
                "none".to_string()
            } else {
                versions.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
            }
        )
    })?;
    let snapshot = audio_policy_snapshot()?;
    let mut source = discover_aidl_primary_config(&snapshot)?;
    let source_is_mounted = path_is_mounted(&source.path);
    let managed_metadata = read_aidl_metadata().ok();
    if source_is_mounted && managed_metadata.is_none() {
        return Err(format!(
            "{} is already mounted by another source; refusing to stack an AIDL writer",
            source.path.display()
        ));
    }
    if let Some((managed_target, _)) = managed_metadata {
        // A bind-mounted target reads back our previous candidate.  Reuse the
        // saved baseline for repeated Apply operations so metadata never
        // drifts away from the immutable source XML.
        if managed_target != source.path {
            return Err(
                "an existing AIDL mount targets a different configuration source".to_string(),
            );
        }
        {
            let original = fs::read(aidl_original_path())
                .map_err(|error| format!("cannot read saved AIDL baseline: {error}"))?;
            if !source_is_mounted && source.xml.as_bytes() != original {
                return Err(
                    "AIDL source changed after configuration; revalidate after the system update"
                        .to_string(),
                );
            }
            source.xml = String::from_utf8(original)
                .map_err(|_| "saved AIDL source is not valid UTF-8".to_string())?;
            source.model = aidl::parse_module_xml(&source.xml)?;
            let runtime = primary_runtime(&snapshot)?;
            let (matches, total) = aidl::topology_match_score(runtime, &source.model);
            if total == 0 || matches != total {
                return Err(format!(
                    "saved AIDL source topology mismatch: {matches}/{total}"
                ));
            }
            source.matches = matches;
            source.total = total;
        }
    }
    let plan = aidl::plan_policy(
        &snapshot,
        &settings.policy,
        settings.sample_rate,
        &settings.bit_depth,
    )?;
    let candidate = aidl::apply_plan_to_module_xml(&source.xml, &source.model.name, &plan)?;
    let candidate_model = aidl::parse_module_xml(&candidate)?;
    let runtime = primary_runtime(&snapshot)?;
    let (matched, total) = aidl::topology_match_score(runtime, &candidate_model);
    validate_candidate(runtime, &source.model, &candidate_model, matched, total)?;

    let service = discover_audio_hal_service();
    write_metadata(&source.path, &service, source.xml.as_bytes())?;
    atomic_write(&aidl_candidate_path(), candidate.as_bytes(), 0o644)?;
    if let Err(error) = set_config_context(&aidl_candidate_path(), &source.path) {
        remove_metadata();
        return Err(error);
    }
    let generated_path = state_root().join("generated/aidl-apply.sh");
    let generated = render_aidl_reload_script(&source.path, Some(&aidl_candidate_path()), &service);
    if let Err(error) = atomic_write(&generated_path, generated.as_bytes(), 0o700) {
        remove_metadata();
        return Err(error);
    }
    let mut execution = match execute_script_locked(&generated, namespace, AIDL_TIMEOUT, lock) {
        Ok(execution) => execution,
        Err(error) => {
            remove_metadata();
            return Err(error);
        }
    };

    if !execution.timed_out && execution.output.status.success() {
        match audio_policy_snapshot().and_then(|loaded| validate_loaded_aidl_plan(&loaded, &plan)) {
            Ok(()) => {}
            Err(error) => {
                execution.output.stderr.extend_from_slice(
                    format!("AIDL post-restart validation failed: {error}\n").as_bytes(),
                );
                execution.output.status = std::process::ExitStatus::from_raw(34 << 8);
                let rollback = render_aidl_reload_script(&source.path, None, &service);
                if let Ok(result) =
                    execute_script_locked(&rollback, namespace, AIDL_RESTART_TIMEOUT, lock)
                {
                    append_execution_output(&mut execution.output, &result.output);
                }
                remove_metadata();
            }
        }
    } else if !execution.timed_out {
        remove_metadata();
    }

    let command = format!(
        "AIDL module-xml apply {} v{}",
        source.path.display(),
        version
    );
    let business_commands = vec![command.clone()];
    finalize_operation(
        OperationContext {
            action: "apply",
            kind: OperationKind::Mutation,
            route,
            command: command_route,
            business_commands: &business_commands,
            script: &generated,
            a2dp_state,
            persistence: Persistence::PolicyApply(&settings),
            reapply_progress: None,
            phased_progress: None,
        },
        execution,
    )
}

fn run_reset(
    lock: &OperationLock,
    namespace: &crate::domain::NamespaceInfo,
    route: &'static str,
    command_route: &str,
    a2dp_state: Option<A2dpState>,
) -> Result<i32, String> {
    let (target, service) = match read_aidl_metadata() {
        Ok(metadata) => metadata,
        Err(_) if !aidl_metadata_path().exists() => {
            crate::state::reset_policy_settings()?;
            return crate::operation::finish_internal_mutation(
                "reset",
                "AIDL policy is already at its system default".to_string(),
            );
        }
        Err(error) => return Err(error),
    };
    let original = fs::read(aidl_original_path())
        .map_err(|error| format!("cannot read saved AIDL source: {error}"))?;
    if !path_is_mounted(&target) {
        let current = fs::read(&target).map_err(|error| {
            format!(
                "cannot read current AIDL source {}: {error}",
                target.display()
            )
        })?;
        if current != original {
            return Err(
                "AIDL source changed after configuration; refusing reset until it is revalidated"
                    .to_string(),
            );
        }
    }
    let generated_path = state_root().join("generated/aidl-reset.sh");
    let generated = render_aidl_reload_script(&target, None, &service);
    atomic_write(&generated_path, generated.as_bytes(), 0o700)?;
    let execution = execute_script_locked(&generated, namespace, AIDL_TIMEOUT, lock)?;
    let code_success = !execution.timed_out && execution.output.status.success();
    if code_success {
        remove_metadata();
    }
    let command = format!("AIDL module-xml reset {}", target.display());
    let business_commands = vec![command];
    finalize_operation(
        OperationContext {
            action: "reset",
            kind: OperationKind::Mutation,
            route,
            command: command_route,
            business_commands: &business_commands,
            script: &generated,
            a2dp_state,
            persistence: Persistence::PolicyReset,
            reapply_progress: None,
            phased_progress: None,
        },
        execution,
    )
}

fn validate_loaded_aidl_plan(
    snapshot: &AudioPolicySnapshot,
    plan: &PolicyPlan,
) -> Result<(), String> {
    for target in &plan.targets {
        let module = snapshot
            .modules
            .iter()
            .find(|module| module.name == target.module)
            .ok_or_else(|| format!("loaded module {} is missing", target.module))?;
        let port = module
            .output_mix_ports
            .iter()
            .find(|port| port.name == target.port)
            .ok_or_else(|| format!("loaded mix port {} is missing", target.port))?;
        let expected_format = match target.pcm_type.as_str() {
            "INT_16_BIT" => "AUDIO_FORMAT_PCM_16_BIT",
            "INT_24_BIT" => "AUDIO_FORMAT_PCM_24_BIT_PACKED",
            "INT_32_BIT" => "AUDIO_FORMAT_PCM_32_BIT",
            "FLOAT_32_BIT" => "AUDIO_FORMAT_PCM_FLOAT",
            value => return Err(format!("unsupported loaded PCM type: {value}")),
        };
        let matches_static = port.profiles.iter().any(|profile| {
            profile.rates.contains(&target.rate)
                && profile.format.as_deref() == Some(expected_format)
        });
        if !matches_static {
            return Err(format!(
                "loaded mix port {} does not expose {} at {} Hz",
                target.port, expected_format, target.rate
            ));
        }
    }
    Ok(())
}

fn append_execution_output(target: &mut Output, source: &Output) {
    target.stdout.extend_from_slice(&source.stdout);
    target.stderr.extend_from_slice(&source.stderr);
}

pub(crate) fn restore() -> Result<i32, String> {
    ensure_state_layout()?;
    let (target, _) = match read_aidl_metadata() {
        Ok(value) => value,
        Err(_) => {
            println!("aidl_restore=not-configured");
            return Ok(0);
        }
    };
    if path_is_mounted(&target) {
        println!("aidl_restore=already-mounted");
        return Ok(0);
    }
    let stored = crate::operation::load_state_preflight(&StateStore::new(), false)?.settings;
    if !stored.policy_configured {
        return Err("AIDL restore requires a successfully saved policy".to_string());
    }
    run_operation(stored.policy, Action::Apply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aidl::{AudioPort, AudioProfile, PortPlan};

    fn expected_plan() -> PolicyPlan {
        PolicyPlan {
            policy: "offload-direct".to_string(),
            targets: vec![PortPlan {
                module: "primary".to_string(),
                port: "direct".to_string(),
                behavior: "direct-pcm",
                rate: 96_000,
                pcm_type: "INT_32_BIT".to_string(),
                dynamic_source: true,
            }],
            skipped: vec![],
        }
    }

    #[test]
    fn post_restart_requires_requested_rate_and_format_even_for_dynamic_ports() {
        let mut snapshot = AudioPolicySnapshot {
            config_source: "AIDL HAL".to_string(),
            modules: vec![AudioModule {
                name: "primary".to_string(),
                output_mix_ports: vec![AudioPort {
                    name: "direct".to_string(),
                    profiles: vec![AudioProfile {
                        dynamic_rate: true,
                        dynamic_format: true,
                        ..AudioProfile::default()
                    }],
                    ..AudioPort::default()
                }],
                ..AudioModule::default()
            }],
        };
        let plan = expected_plan();
        assert!(validate_loaded_aidl_plan(&snapshot, &plan).is_err());
        snapshot.modules[0].output_mix_ports[0].profiles[0] = AudioProfile {
            format: Some("AUDIO_FORMAT_PCM_32_BIT".to_string()),
            rates: vec![96_000],
            ..AudioProfile::default()
        };
        assert!(validate_loaded_aidl_plan(&snapshot, &plan).is_ok());
        snapshot.modules[0].output_mix_ports[0].profiles[0].rates = vec![48_000];
        assert!(validate_loaded_aidl_plan(&snapshot, &plan).is_err());
        snapshot.modules[0].output_mix_ports[0].profiles[0].rates = vec![96_000];
        snapshot.modules[0].name = "usb".to_string();
        assert!(validate_loaded_aidl_plan(&snapshot, &plan).is_err());
    }

    #[test]
    fn unsupported_template_switches_are_not_silently_ignored() {
        assert!(validate_aidl_settings(&Settings::default()).is_ok());
        let mut settings = Settings::default();
        settings.force_usbv2 = true;
        assert!(validate_aidl_settings(&settings).is_err());
        settings.force_usbv2 = false;
        settings.drc = true;
        assert!(validate_aidl_settings(&settings).is_err());
    }

    #[test]
    fn ambiguous_or_unsupported_aidl_versions_cannot_apply() {
        assert_eq!(supported_audio_core_aidl_version(&[2]), Some(2));
        assert_eq!(supported_audio_core_aidl_version(&[3]), Some(3));
        for versions in [vec![], vec![1], vec![4], vec![2, 3]] {
            assert_eq!(supported_audio_core_aidl_version(&versions), None);
        }
    }
}
