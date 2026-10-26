//! Experimental device-compatible policy templates. Never edits upstream templates.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};

use crate::domain::Settings;
use crate::paths::{atomic_write, state_root};

const GENERATED: &str = "/data/local/tmp/audio_conf_generated.xml";
const MARKER: &str = "<!-- usbsrctl:audio-device-dynamic:v3 -->";
const BLUETOOTH_MARKER: &str = "<!-- usbsrctl:bluetooth-dynamic:v2 -->";
const LEGACY_MARKER: &str = "<!-- usbsrctl:direct-pcm-dynamic:v1 -->";
const BT_PLACEHOLDER: &str = "dynamic_bluetooth_placeholder";
const A2DP: &[&str] = &[
    "AUDIO_DEVICE_OUT_BLUETOOTH_A2DP",
    "AUDIO_DEVICE_OUT_BLUETOOTH_A2DP_HEADPHONES",
    "AUDIO_DEVICE_OUT_BLUETOOTH_A2DP_SPEAKER",
];
const SPEAKERS: &[&str] = &["AUDIO_DEVICE_OUT_SPEAKER", "AUDIO_DEVICE_OUT_SPEAKER_SAFE"];
const SPEAKER_SLOTS: &[(&str, &str, &str)] = &[
    (
        "AUDIO_DEVICE_OUT_SPEAKER",
        "usbsrctl:inherit-speaker-device",
        "usbsrctl:inherit-speaker-route",
    ),
    (
        "AUDIO_DEVICE_OUT_SPEAKER_SAFE",
        "usbsrctl:inherit-speaker-safe-device",
        "usbsrctl:inherit-speaker-safe-route",
    ),
];

pub(crate) fn generated_is_dynamic() -> bool {
    fs::read_to_string(GENERATED).is_ok_and(|text| {
        text.contains(MARKER) || text.contains(BLUETOOTH_MARKER) || text.contains(LEGACY_MARKER)
    })
}

pub(crate) fn template_name(policy: &str) -> Option<&'static str> {
    match policy {
        "bypass-dynamic" | "safest-auto-dynamic" => Some("bypass_offload_dynamic_template.xml"),
        "bypass-safer-dynamic" | "safe-dynamic" => {
            Some("bypass_offload_safer_dynamic_template.xml")
        }
        "offload-dynamic" => Some("offload_dynamic_template.xml"),
        "offload-hifi-playback-dynamic" => Some("offload_hifi_playback_dynamic_template.xml"),
        "offload-direct-dynamic" => Some("offload_direct_dynamic_template.xml"),
        "offload-safer-dynamic" => Some("offload_safer_dynamic_template.xml"),
        "legacy-dynamic" => Some("legacy_dynamic_template.xml"),
        "safest-dynamic" => Some("safest_dynamic_template.xml"),
        _ => None,
    }
}

pub(crate) fn is_dynamic_policy(policy: &str) -> bool {
    template_name(policy).is_some()
}

fn parse(text: &str) -> Result<Document<'_>, String> {
    Document::parse(text).map_err(|e| format!("invalid audio policy XML: {e}"))
}
fn unique<'a, 'i>(
    nodes: impl Iterator<Item = Node<'a, 'i>>,
    label: &str,
) -> Result<Node<'a, 'i>, String> {
    let values: Vec<_> = nodes.collect();
    if values.len() != 1 {
        return Err(format!("expected one {label}, found {}", values.len()));
    }
    Ok(values[0])
}
fn module<'a, 'i>(doc: &'a Document<'i>, name: &str) -> Result<Node<'a, 'i>, String> {
    unique(
        doc.descendants()
            .filter(|n| n.has_tag_name("module") && n.attribute("name") == Some(name)),
        name,
    )
}
fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>, String> {
    unique(node.children().filter(|n| n.has_tag_name(name)), name)
}
fn attr<'a>(node: Node<'_, 'a>, key: &str) -> Result<String, String> {
    node.attribute(key)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {key} on {}", node.tag_name().name()))
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn edit(text: &str, mut edits: Vec<(Range<usize>, String)>) -> Result<String, String> {
    edits.sort_by_key(|(r, _)| r.start);
    if edits.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return Err("overlapping XML edits".into());
    }
    let mut output = text.to_owned();
    for (range, value) in edits.into_iter().rev() {
        output.replace_range(range, &value);
    }
    Ok(output)
}
fn renamed_node(text: &str, node: Node<'_, '_>, key: &str, value: &str) -> Result<String, String> {
    let attribute = node
        .attributes()
        .find(|a| a.name() == key)
        .ok_or_else(|| format!("missing {key}"))?;
    let range = node.range();
    let value_range = attribute.range_value();
    edit(
        &text[range.clone()],
        vec![(
            value_range.start - range.start..value_range.end - range.start,
            escape(value),
        )],
    )
}
fn no_includes(node: Node<'_, '_>) -> Result<(), String> {
    if node
        .descendants()
        .any(|n| n.has_tag_name(("http://www.w3.org/2001/XInclude", "include")))
    {
        return Err("dynamic compatibility policies require inline Primary/Bluetooth modules; nested XInclude is not supported".into());
    }
    Ok(())
}

fn anchored_element<'a, 'i>(
    container: Node<'a, 'i>,
    marker: &str,
    element: &str,
) -> Result<Option<Node<'a, 'i>>, String> {
    let anchors: Vec<_> = container
        .children()
        .filter(|node| node.is_comment() && node.text().is_some_and(|text| text.trim() == marker))
        .collect();
    if anchors.len() > 1 {
        return Err(format!("duplicate dynamic template anchor: {marker}"));
    }
    let Some(anchor) = anchors.first() else {
        return Ok(None);
    };
    let node = anchor
        .next_siblings()
        .find(Node::is_element)
        .ok_or_else(|| format!("dynamic template anchor has no element: {marker}"))?;
    if !node.has_tag_name(element) {
        return Err(format!(
            "dynamic template anchor {marker} must precede {element}"
        ));
    }
    Ok(Some(node))
}

fn a2dp_device(node: Node<'_, '_>) -> bool {
    node.has_tag_name("devicePort")
        && node
            .attribute("type")
            .is_some_and(|kind| A2DP.contains(&kind))
        && node.attribute("role") == Some("sink")
}

fn bluetooth_media_device(node: Node<'_, '_>) -> bool {
    node.has_tag_name("devicePort")
        && node
            .attribute("type")
            .is_some_and(|kind| A2DP.contains(&kind) || kind.starts_with("AUDIO_DEVICE_OUT_BLE_"))
}

fn speaker_device(node: Node<'_, '_>) -> bool {
    node.has_tag_name("devicePort")
        && node
            .attribute("type")
            .is_some_and(|kind| SPEAKERS.contains(&kind))
        && node.attribute("role") == Some("sink")
}

fn flags(node: Node<'_, '_>) -> BTreeSet<String> {
    node.attribute("flags")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn optional_primary_source(name: &str) -> bool {
    matches!(
        name,
        "direct_pcm"
            | "compressed_offload"
            | "mmap_no_irq_out"
            | "haptic"
            | "voip_rx"
            | "hifi_playback"
    )
}

fn mapped_primary_sources(
    stock: &str,
    source_mixes: Node<'_, '_>,
    target_mixes: Node<'_, '_>,
    source_route: Node<'_, '_>,
    inherit_mix_ports: bool,
    inherited_mixes: &mut BTreeSet<String>,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<Vec<String>, String> {
    let mut mapped = Vec::new();
    for name in attr(source_route, "sources")?.split(',').map(str::trim) {
        let source_mix = unique(
            source_mixes.children().filter(|node| {
                node.has_tag_name("mixPort")
                    && node.attribute("name") == Some(name)
                    && node.attribute("role") == Some("source")
            }),
            "system route source",
        )?;
        let mapped_name = if target_mixes.children().any(|node| {
            node.has_tag_name("mixPort")
                && node.attribute("name") == Some(name)
                && node.attribute("role") == Some("source")
        }) {
            name
        } else if name == "deep_buffer"
            && target_mixes.children().any(|node| {
                node.has_tag_name("mixPort")
                    && node.attribute("name") == Some("deep buffer")
                    && node.attribute("role") == Some("source")
            })
        {
            "deep buffer"
        } else if optional_primary_source(name) {
            continue;
        } else {
            return Err(format!(
                "cannot map system Primary route source {name}; original policy retained"
            ));
        };
        let target_mix = unique(
            target_mixes.children().filter(|node| {
                node.has_tag_name("mixPort")
                    && node.attribute("name") == Some(mapped_name)
                    && node.attribute("role") == Some("source")
            }),
            "mapped route source",
        )?;
        if inherited_mixes.contains(mapped_name) {
            // A previous inherited route already replaced this shared mixPort.
        } else if inherit_mix_ports || name == "voip_rx" {
            inherited_mixes.insert(mapped_name.to_owned());
            edits.push((
                target_mix.range(),
                renamed_node(stock, source_mix, "name", mapped_name)?,
            ));
        } else if flags(source_mix) != flags(target_mix) {
            return Err(format!(
                "incompatible output flags for {name} -> {mapped_name}"
            ));
        }
        mapped.push(mapped_name.to_owned());
    }
    if mapped.is_empty() {
        return Err("system route has no compatible template source".into());
    }
    Ok(mapped)
}

/// Inherit the stock speaker and Bluetooth topology into a separate template
/// copy while preserving the template's supported output set and USB behavior.
fn inherit(stock: &str, template: &str) -> Result<String, String> {
    let source = parse(stock)?;
    let target = parse(template)?;
    if source.root_element().attribute("version") != Some("7.0") {
        return Err(
            "dynamic compatibility policies support HIDL policy XML version 7.0 only".into(),
        );
    }
    let sp = module(&source, "primary")?;
    let tp = module(&target, "primary")?;
    no_includes(sp)?;
    let source_ports = child(sp, "devicePorts")?;
    let source_mixes = child(sp, "mixPorts")?;
    let source_routes = child(sp, "routes")?;
    let target_ports = child(tp, "devicePorts")?;
    let target_mixes = child(tp, "mixPorts")?;
    let target_routes = child(tp, "routes")?;
    let mut edits = Vec::new();
    let mut inherited_mixes = BTreeSet::new();

    for (device_type, device_anchor, route_anchor) in SPEAKER_SLOTS {
        let source: Vec<_> = source_ports
            .children()
            .filter(|node| speaker_device(*node) && node.attribute("type") == Some(*device_type))
            .collect();
        let target: Vec<_> = target_ports
            .children()
            .filter(|node| speaker_device(*node) && node.attribute("type") == Some(*device_type))
            .collect();
        let anchored_device = anchored_element(target_ports, device_anchor, "devicePort")?;
        let anchored_route = anchored_element(target_routes, route_anchor, "route")?;
        if target.is_empty() {
            if anchored_device.is_some() || anchored_route.is_some() {
                return Err(format!(
                    "dynamic template anchors for missing {device_type} slot"
                ));
            }
            if *device_type == "AUDIO_DEVICE_OUT_SPEAKER" || source.len() > 1 {
                return Err(format!(
                    "expected one {device_type} in system/template Primary, found {}/{}",
                    source.len(),
                    target.len()
                ));
            }
            continue;
        }
        if target.len() != 1
            || anchored_device != target.first().copied()
            || anchored_route.is_none()
        {
            return Err(format!(
                "invalid dynamic template anchors for {device_type}"
            ));
        }
        let target_device = target[0];
        let target_tag = attr(target_device, "tagName")?;
        let target_route = unique(
            target_routes.children().filter(|node| {
                node.has_tag_name("route") && node.attribute("sink") == Some(target_tag.as_str())
            }),
            "template speaker route",
        )?;
        if anchored_route != Some(target_route) {
            return Err(format!(
                "dynamic template route anchor does not identify {target_tag}"
            ));
        }
        match source.as_slice() {
            [source_device] => {
                let source_tag = attr(*source_device, "tagName")?;
                edits.push((
                    target_device.range(),
                    renamed_node(stock, *source_device, "tagName", &target_tag)?,
                ));
                let source_route = unique(
                    source_routes.children().filter(|node| {
                        node.has_tag_name("route")
                            && node.attribute("sink") == Some(source_tag.as_str())
                    }),
                    "system speaker route",
                )?;
                if source_route.attribute("type") != Some("mix") {
                    return Err("unsupported speaker route type".into());
                }
                let mapped = mapped_primary_sources(
                    stock,
                    source_mixes,
                    target_mixes,
                    source_route,
                    true,
                    &mut inherited_mixes,
                    &mut edits,
                )?;
                edits.push((
                    target_route.range(),
                    format!(
                        "<route type=\"mix\" sink=\"{}\" sources=\"{}\"/>",
                        escape(&target_tag),
                        escape(&mapped.join(","))
                    ),
                ));
            }
            [] if *device_type == "AUDIO_DEVICE_OUT_SPEAKER_SAFE" => {}
            _ => {
                return Err(format!(
                    "expected one {device_type} in system Primary, found {}",
                    source.len()
                ));
            }
        }
    }

    let target_a2dp: Vec<_> = target_ports
        .children()
        .filter(|node| a2dp_device(*node))
        .collect();
    let primary_a2dp = match target_a2dp.len() {
        0 => false,
        3 if A2DP.iter().all(|kind| {
            target_a2dp
                .iter()
                .filter(|node| node.attribute("type") == Some(*kind))
                .count()
                == 1
        }) =>
        {
            true
        }
        count => {
            return Err(format!(
                "template Primary has partial A2DP topology ({count}/3); original policy retained"
            ));
        }
    };

    if primary_a2dp {
        for device_type in A2DP {
            let matches =
                |n: &Node<'_, '_>| a2dp_device(*n) && n.attribute("type") == Some(*device_type);
            let sd = unique(source_ports.children().filter(matches), device_type)?;
            let td = unique(target_ports.children().filter(matches), device_type)?;
            let formats = attr(sd, "encodedFormats")?;
            if !formats
                .split_whitespace()
                .any(|f| f.starts_with("AUDIO_FORMAT_") && f != "AUDIO_FORMAT_FORCE_AOSP")
            {
                return Err(format!("{device_type}: no system Primary offload codecs"));
            }
            let source_tag = attr(sd, "tagName")?;
            let target_tag = attr(td, "tagName")?;
            edits.push((td.range(), renamed_node(stock, sd, "tagName", &target_tag)?));
            let sr = unique(
                source_routes.children().filter(|n| {
                    n.has_tag_name("route") && n.attribute("sink") == Some(source_tag.as_str())
                }),
                "system A2DP route",
            )?;
            let tr = unique(
                target_routes.children().filter(|n| {
                    n.has_tag_name("route") && n.attribute("sink") == Some(target_tag.as_str())
                }),
                "template A2DP route",
            )?;
            if sr.attribute("type") != Some("mix") {
                return Err("unsupported A2DP route type".into());
            }
            let mapped = mapped_primary_sources(
                stock,
                source_mixes,
                target_mixes,
                sr,
                false,
                &mut inherited_mixes,
                &mut edits,
            )?;
            edits.push((
                tr.range(),
                format!(
                    "<route type=\"mix\" sink=\"{}\" sources=\"{}\"/>",
                    escape(&target_tag),
                    escape(&mapped.join(","))
                ),
            ));
        }
    }
    // Preserve vendor fallback semantics (e.g. FORCE_AOSP) and dynamic profiles.
    // No generic module-name guess or codec list is substituted here.
    let bluetooth: Vec<_> = source
        .descendants()
        .filter(|n| {
            n.has_tag_name("module") && *n != sp && n.descendants().any(bluetooth_media_device)
        })
        .collect();
    if !primary_a2dp
        && A2DP.iter().any(|kind| {
            bluetooth
                .iter()
                .flat_map(|node| node.descendants())
                .filter(|node| a2dp_device(*node) && node.attribute("type") == Some(*kind))
                .count()
                != 1
        })
    {
        return Err("system dedicated Bluetooth module lacks a complete A2DP topology".into());
    }
    for node in &bluetooth {
        no_includes(*node)?;
    }
    let bt = module(&target, BT_PLACEHOLDER)?;
    edits.push((
        bt.range(),
        bluetooth
            .iter()
            .map(|n| &stock[n.range()])
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    let output = edit(template, edits)?;
    validate_graph(&output)?;
    Ok(format!("{output}\n{MARKER}\n"))
}

fn validate_graph(text: &str) -> Result<(), String> {
    let doc = parse(text)?;
    let mut modules = BTreeSet::new();
    for m in doc.descendants().filter(|n| n.has_tag_name("module")) {
        let name = attr(m, "name")?;
        if !modules.insert(name.clone()) {
            return Err(format!("duplicate module: {name}"));
        }
        let mut ports = BTreeMap::new();
        for p in m
            .descendants()
            .filter(|n| n.has_tag_name("mixPort") || n.has_tag_name("devicePort"))
        {
            let tag = attr(
                p,
                if p.has_tag_name("mixPort") {
                    "name"
                } else {
                    "tagName"
                },
            )?;
            if ports.insert(tag.clone(), attr(p, "role")?).is_some() {
                return Err(format!("duplicate port: {name}/{tag}"));
            }
        }
        for r in m.descendants().filter(|n| n.has_tag_name("route")) {
            let sink = attr(r, "sink")?;
            if ports.get(&sink).map(String::as_str) != Some("sink") {
                return Err(format!("invalid route sink: {name}/{sink}"));
            }
            for source in attr(r, "sources")?.split(',').map(str::trim) {
                if ports.get(source).map(String::as_str) != Some("source") {
                    return Err(format!("invalid route source: {name}/{source}"));
                }
            }
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    let data =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if data.len() > 2 * 1024 * 1024 {
        return Err(format!("XML/state file too large: {}", path.display()));
    }
    Ok(data)
}
fn command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn verify_policy_view(root: &Path, target: &Path, expected: &str) -> Result<(), String> {
    let relative = target
        .strip_prefix("/")
        .map_err(|_| format!("policy target is not absolute: {}", target.display()))?;
    let visible_path = root.join(relative);
    let visible = read(&visible_path)
        .map_err(|error| format!("audioserver cannot read {}: {error}", target.display()))?;
    if visible != expected {
        return Err(format!(
            "audioserver sees a different policy at {}: expected {}, found {}",
            target.display(),
            digest(expected),
            digest(&visible)
        ));
    }
    Ok(())
}
fn verify_audioserver_policy(pid: u32, target: &Path, expected: &str) -> Result<(), String> {
    verify_policy_view(
        &PathBuf::from(format!("/proc/{pid}/root")),
        target,
        expected,
    )
}
fn mounted_roots(mountinfo: &str, target: &str) -> Vec<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            (fields.get(4).copied() == Some(target)).then(|| fields[3].to_string())
        })
        .collect()
}
fn snapshot_path(target: &Path, identity: &str, directory: &Path) -> std::path::PathBuf {
    let key = digest(&format!("{}\n{identity}", target.display()));
    directory.join(format!("{key}.xml"))
}
fn is_snapshot_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let stem = name
        .strip_suffix(".xml")
        .or_else(|| name.strip_suffix(".sha256"));
    stem.is_some_and(|stem| stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
fn prune_stale_snapshots(directory: &Path, snapshot: &Path) -> Result<(), String> {
    let checksum = snapshot.with_extension("sha256");
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect {}: {error}", directory.display()))?;
        let path = entry.path();
        if path == snapshot || path == checksum || !is_snapshot_file(&path) {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
            .is_file()
        {
            fs::remove_file(&path).map_err(|error| {
                format!("cannot remove stale snapshot {}: {error}", path.display())
            })?;
        }
    }
    Ok(())
}
fn baseline(
    target: &Path,
    roots: &[String],
    identity: &str,
    directory: &Path,
) -> Result<String, String> {
    let snapshot = snapshot_path(target, identity, directory);
    if roots.is_empty() {
        let original = read(target)?;
        if original.contains(MARKER)
            || original.contains(BLUETOOTH_MARKER)
            || original.contains(LEGACY_MARKER)
        {
            return Err("dynamic policy is not a valid system baseline; reset first".into());
        }
        parse(&original)?;
        atomic_write(&snapshot, original.as_bytes(), 0o600)?;
        atomic_write(
            &snapshot.with_extension("sha256"),
            digest(&original).as_bytes(),
            0o600,
        )?;
        prune_stale_snapshots(directory, &snapshot)?;
        return Ok(original);
    }
    if roots.len() != 1 || !roots[0].ends_with("/audio_conf_generated.xml") {
        return Err(
            "audio policy has an unrecognized overlay; reset it before using a dynamic compatibility policy"
                .into(),
        );
    }
    if !snapshot.is_file() {
        return Err("no original policy snapshot for this boot/ROM; reset the existing policy before first dynamic apply".into());
    }
    let original = read(&snapshot)?;
    if read(&snapshot.with_extension("sha256"))? != digest(&original) {
        return Err("original policy snapshot checksum mismatch; reset first".into());
    }
    prune_stale_snapshots(directory, &snapshot)?;
    Ok(original)
}
fn include_path(stock: &str, target: &Path, suffix: &str) -> Result<String, String> {
    let doc = parse(stock)?;
    let includes: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name(("http://www.w3.org/2001/XInclude", "include")))
        .filter_map(|n| n.attribute("href"))
        .filter(|href| href.ends_with(suffix))
        .collect();
    if includes.len() != 1 {
        return Err(format!("expected one system {suffix} include"));
    }
    let path = target
        .parent()
        .ok_or("missing policy directory")?
        .join(includes[0]);
    let path =
        fs::canonicalize(&path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    if !["/vendor/", "/odm/", "/system/", "/system_ext/", "/product/"]
        .iter()
        .any(|prefix| path.to_string_lossy().starts_with(prefix))
    {
        return Err("volume include outside system partitions".into());
    }
    Ok(path.to_string_lossy().into_owned())
}
fn render(
    template: &str,
    settings: &Settings,
    usb: &str,
    volume: &str,
    default_volume: &str,
) -> Result<String, String> {
    let format = match settings.bit_depth.as_str() {
        "16" => "AUDIO_FORMAT_PCM_16_BIT",
        "24" => "AUDIO_FORMAT_PCM_24_BIT_PACKED",
        "32" => "AUDIO_FORMAT_PCM_32_BIT",
        "float" => "AUDIO_FORMAT_PCM_FLOAT",
        _ => return Err("unsupported PCM format".into()),
    };
    let mut output = template.to_owned();
    for (key, value) in [
        ("%DRC_ENABLED%", if settings.drc { "true" } else { "false" }),
        ("%USB_MODULE%", usb),
        ("%BT_MODULE%", BT_PLACEHOLDER),
        ("%SAMPLING_RATE%", &settings.sample_rate.to_string()),
        ("%AUDIO_FORMAT%", format),
        ("%VOLUME_FILE%", volume),
        ("%DEFAULT_VOLUME_FILE%", default_volume),
    ] {
        output = output.replace(key, &escape(value));
    }
    if output.contains('%') {
        return Err("unresolved dynamic template placeholder".into());
    }
    Ok(output)
}

pub(crate) fn apply(settings: &Settings, module_dir: &Path) -> Result<(), String> {
    crate::scripts::validate_settings(settings, module_dir)?;
    let template_name = template_name(&settings.policy)
        .ok_or("internal dynamic entry requires a dynamic compatibility policy")?;
    let ns = crate::android::namespace_info();
    if ns.is_global() != Some(true) {
        return Err("dynamic policy must run in the global mount namespace".into());
    }
    let audio_pid = ns
        .audio_pid
        .ok_or("audioserver is unavailable for policy visibility verification")?;
    let dump = command("dumpsys", &["media.audio_policy"])?;
    let path = dump
        .lines()
        .find_map(|l| l.trim().strip_prefix("Config source: "))
        .ok_or("audio policy source unavailable")?;
    let target =
        fs::canonicalize(path).map_err(|e| format!("cannot resolve policy source: {e}"))?;
    let target_text = target.to_string_lossy();
    if !["/vendor/", "/odm/"]
        .iter()
        .any(|prefix| target_text.starts_with(prefix))
    {
        return Err("dynamic compatibility policies require a vendor/odm HIDL XML policy".into());
    }
    let roots = mounted_roots(&read(Path::new("/proc/self/mountinfo"))?, &target_text);
    let fingerprint = command("getprop", &["ro.build.fingerprint"])?;
    let boot = read(Path::new("/proc/sys/kernel/random/boot_id"))?;
    if fingerprint.is_empty() || boot.trim().is_empty() {
        return Err("cannot identify ROM/boot for policy snapshot".into());
    }
    let directory = state_root().join("direct-pcm-dynamic");
    let stock = baseline(
        &target,
        &roots,
        &format!("{fingerprint}\n{boot}"),
        &directory,
    )?;
    let volume = include_path(&stock, &target, "audio_policy_volumes.xml")?;
    let default_volume = include_path(&stock, &target, "default_volume_tables.xml")?;
    let exists_hal = |name: &str| {
        ["/vendor/lib64/hw", "/vendor/lib/hw"].iter().any(|dir| {
            Path::new(dir)
                .join(format!("audio.{name}.default.so"))
                .is_file()
        })
    };
    let usb = if settings.force_usbv2 || (!exists_hal("usb") && exists_hal("usbv2")) {
        "usbv2"
    } else {
        "usb"
    };
    let template = read(&module_dir.join("core/templates").join(template_name))?;
    let rendered = render(&template, settings, usb, &volume, &default_volume)?;
    let generated = inherit(&stock, &rendered)?;
    let candidate = directory.join("candidate.xml");
    atomic_write(&candidate, generated.as_bytes(), 0o644)?;
    // Validate and persist the candidate before touching any live mount.
    // The shared generated path keeps the existing reset/uninstall compatible.
    let old = if roots.is_empty() {
        None
    } else {
        Some(read(&target)?)
    };
    if old.is_some() {
        command("umount", &[&target_text])?;
    }
    let restore_previous = |previous: &str| -> Result<(), String> {
        atomic_write(Path::new(GENERATED), previous.as_bytes(), 0o644)?;
        command("chcon", &["u:object_r:vendor_configs_file:s0", GENERATED])?;
        command("mount", &["-o", "bind", GENERATED, &target_text])?;
        Ok(())
    };
    let prepare = || -> Result<(), String> {
        atomic_write(Path::new(GENERATED), generated.as_bytes(), 0o644)?;
        command("chcon", &["u:object_r:vendor_configs_file:s0", GENERATED])?;
        Ok(())
    };
    if let Err(error) = prepare() {
        if let Some(old) = old.as_deref() {
            restore_previous(old)
                .map_err(|restore| format!("{error}; previous mount restore failed: {restore}"))?;
        }
        return Err(error);
    }
    if let Err(error) = command("mount", &["-o", "bind", GENERATED, &target_text]) {
        if let Some(old) = old.as_deref() {
            restore_previous(old)
                .map_err(|restore| format!("{error}; previous mount restore failed: {restore}"))?;
        }
        return Err(error);
    }
    if let Err(error) = verify_audioserver_policy(audio_pid, &target, &generated) {
        command("umount", &[&target_text])
            .map_err(|cleanup| format!("{error}; new mount cleanup failed: {cleanup}"))?;
        if let Some(old) = old.as_deref() {
            restore_previous(old)
                .map_err(|restore| format!("{error}; previous mount restore failed: {restore}"))?;
        }
        return Err(error);
    }
    println!("dynamic_direct=1\ndynamic_policy={}\ndynamic_template={}\ndynamic_source={}\ndynamic_source_sha256={}\ndynamic_candidate={}\ndynamic_xml_sha256={}", settings.policy, template_name, target.display(), digest(&stock), candidate.display(), digest(&generated));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const STOCK: &str = include_str!("../tests/fixtures/thor-stock-policy.xml");
    const DYNAMIC_POLICIES: &[&str] = &[
        "bypass-dynamic",
        "bypass-safer-dynamic",
        "offload-dynamic",
        "offload-hifi-playback-dynamic",
        "offload-direct-dynamic",
        "offload-safer-dynamic",
        "legacy-dynamic",
        "safe-dynamic",
        "safest-dynamic",
        "safest-auto-dynamic",
    ];

    // Read an exact new-file payload installed by the build patch.
    fn template_named(name: &str) -> String {
        let patch = include_str!("../../patches/0005-add-dynamic-device-policy-templates.patch");
        let header = format!("diff --git a/templates/{name} b/templates/{name}");
        patch
            .split_once(&header)
            .unwrap_or_else(|| panic!("missing patch section for {name}"))
            .1
            .lines()
            .skip_while(|line| !line.starts_with("@@ "))
            .skip(1)
            .take_while(|line| !line.starts_with("diff --git "))
            .map(|line| format!("{}\n", line.strip_prefix('+').expect("new-file patch line")))
            .collect()
    }
    fn rendered_named(name: &str) -> String {
        render(
            &template_named(name),
            &Settings::default(),
            "usb",
            "/vendor/etc/audio_policy_volumes.xml",
            "/vendor/etc/default_volume_tables.xml",
        )
        .unwrap()
    }
    fn rendered() -> String {
        rendered_named("offload_direct_dynamic_template.xml")
    }

    #[test]
    fn every_dynamic_policy_maps_to_a_valid_inherited_template() {
        for policy in DYNAMIC_POLICIES {
            let name = template_name(policy).unwrap();
            let generated = inherit(STOCK, &rendered_named(name))
                .unwrap_or_else(|error| panic!("{policy}/{name}: {error}"));
            assert!(generated.contains(MARKER), "{policy}");
            assert!(module(&parse(&generated).unwrap(), "bluetooth_qti").is_ok());
        }
        assert!(template_name("auto").is_none());
        assert!(template_name("usb").is_none());
    }
    #[test]
    fn thor_inherits_lhdc_profiles_routes_and_vendor_fallback() {
        let generated = inherit(STOCK, &rendered()).unwrap();
        let doc = parse(&generated).unwrap();
        let primary = module(&doc, "primary").unwrap();
        for p in primary.descendants().filter(|n| {
            n.has_tag_name("devicePort") && n.attribute("type").is_some_and(|t| A2DP.contains(&t))
        }) {
            assert!(p
                .attribute("encodedFormats")
                .unwrap()
                .contains("AUDIO_FORMAT_LHDC"));
            assert_eq!(
                child(p, "profile").unwrap().attribute("format"),
                Some("AUDIO_FORMAT_PCM_16_BIT")
            );
        }
        assert!(
            generated.contains("primary output,deep buffer,direct_pcm,compressed_offload,voip_rx")
        );
        let bt = module(&doc, "bluetooth_qti").unwrap();
        assert!(generated[bt.range()].contains("AUDIO_FORMAT_FORCE_AOSP"));
        let a2dp = bt
            .descendants()
            .find(|n| n.has_tag_name("mixPort") && n.attribute("name") == Some("a2dp output"))
            .unwrap();
        assert_eq!(a2dp.children().filter(|n| n.is_element()).count(), 0);
    }
    #[test]
    fn inherits_stock_speaker_path_and_keeps_usb_template_nodes() {
        let rendered = rendered();
        let generated = inherit(STOCK, &rendered).unwrap();
        let before = parse(&rendered).unwrap();
        let after = parse(&generated).unwrap();
        let find_usb_playback = |document: &Document<'_>| {
            document
                .descendants()
                .find(|node| {
                    node.has_tag_name("mixPort") && node.attribute("name") == Some("usb_playback")
                })
                .unwrap()
                .range()
        };
        assert_eq!(
            &rendered[find_usb_playback(&before)],
            &generated[find_usb_playback(&after)]
        );

        let primary = module(&after, "primary").unwrap();
        let speaker = primary
            .descendants()
            .find(|node| {
                speaker_device(*node) && node.attribute("type") == Some("AUDIO_DEVICE_OUT_SPEAKER")
            })
            .unwrap();
        assert_eq!(speaker.attribute("address"), Some(""));
        assert_eq!(
            child(speaker, "profile").unwrap().attribute("format"),
            Some("AUDIO_FORMAT_PCM_16_BIT")
        );
        let primary_output = primary
            .descendants()
            .find(|node| {
                node.has_tag_name("mixPort") && node.attribute("name") == Some("primary output")
            })
            .unwrap();
        let profile = child(primary_output, "profile").unwrap();
        assert_eq!(
            profile.attribute("format"),
            Some("AUDIO_FORMAT_PCM_24_BIT_PACKED")
        );
        assert_eq!(profile.attribute("samplingRates"), Some("48000"));
        let deep_buffer = primary
            .descendants()
            .find(|node| {
                node.has_tag_name("mixPort") && node.attribute("name") == Some("deep buffer")
            })
            .unwrap();
        assert_eq!(
            child(deep_buffer, "profile").unwrap().attribute("format"),
            Some("AUDIO_FORMAT_PCM_24_BIT_PACKED")
        );
        let route = primary
            .descendants()
            .find(|node| node.has_tag_name("route") && node.attribute("sink") == Some("Speaker"))
            .unwrap();
        assert_eq!(
            route.attribute("sources"),
            Some(
                "primary output,deep buffer,direct_pcm,compressed_offload,voip_rx,mmap_no_irq_out"
            )
        );
    }
    #[test]
    fn speaker_inheritance_respects_each_templates_output_set() {
        let generated = inherit(
            STOCK,
            &rendered_named("bypass_offload_safer_dynamic_template.xml"),
        )
        .unwrap();
        let doc = parse(&generated).unwrap();
        let primary = module(&doc, "primary").unwrap();
        let route = primary
            .descendants()
            .find(|node| node.has_tag_name("route") && node.attribute("sink") == Some("Speaker"))
            .unwrap();
        assert_eq!(
            route.attribute("sources"),
            Some("primary output,deep buffer,voip_rx")
        );
        assert!(!route.attribute("sources").unwrap().contains("direct_pcm"));
        assert!(!route
            .attribute("sources")
            .unwrap()
            .contains("compressed_offload"));
    }
    #[test]
    fn rejects_missing_or_ambiguous_primary_speaker() {
        let missing = STOCK.replacen(
            "type=\"AUDIO_DEVICE_OUT_SPEAKER\" address=\"\"",
            "type=\"AUDIO_DEVICE_OUT_LINE\" address=\"\"",
            1,
        );
        assert!(inherit(&missing, &rendered())
            .unwrap_err()
            .contains("expected one AUDIO_DEVICE_OUT_SPEAKER"));

        let duplicate = STOCK.replacen(
            "            <devicePorts>\n                <!-- Output devices declaration",
            "            <devicePorts>\n                <devicePort tagName=\"Second Speaker\" type=\"AUDIO_DEVICE_OUT_SPEAKER\" role=\"sink\"/>\n                <!-- Output devices declaration",
            1,
        );
        assert!(inherit(&duplicate, &rendered())
            .unwrap_err()
            .contains("expected one AUDIO_DEVICE_OUT_SPEAKER"));
    }
    #[test]
    fn requires_explicit_speaker_template_anchors() {
        let missing = rendered().replace("<!-- usbsrctl:inherit-speaker-device -->", "");
        assert!(inherit(STOCK, &missing)
            .unwrap_err()
            .contains("invalid dynamic template anchors"));

        let duplicate = rendered().replace(
            "<!-- usbsrctl:inherit-speaker-route -->",
            "<!-- usbsrctl:inherit-speaker-route --><!-- usbsrctl:inherit-speaker-route -->",
        );
        assert!(inherit(STOCK, &duplicate)
            .unwrap_err()
            .contains("duplicate dynamic template anchor"));
    }
    #[test]
    fn rejects_unknown_sources_duplicate_ports_and_nested_includes() {
        assert!(inherit(
            &STOCK.replace("deep_buffer", "unknown_vendor_output"),
            &rendered()
        )
        .unwrap_err()
        .contains("cannot map"));
        let duplicate = STOCK.replace("<devicePorts>", "<devicePorts><devicePort tagName=\"duplicate\" type=\"AUDIO_DEVICE_OUT_BLUETOOTH_A2DP\" role=\"sink\"/>");
        assert!(inherit(&duplicate, &rendered()).is_err());
        let included = STOCK.replacen(
            "<mixPorts>",
            "<xi:include href=\"ports.xml\"/><mixPorts>",
            1,
        );
        assert!(inherit(&included, &rendered())
            .unwrap_err()
            .contains("XInclude"));

        let partial = rendered().replace(
            "AUDIO_DEVICE_OUT_BLUETOOTH_A2DP_SPEAKER",
            "AUDIO_DEVICE_OUT_BLE_SPEAKER",
        );
        assert!(inherit(STOCK, &partial)
            .unwrap_err()
            .contains("partial A2DP topology"));
    }
    #[test]
    fn rejects_missing_codec_and_dangling_template_route() {
        assert!(inherit(
            &STOCK.replace("encodedFormats=", "removedFormats="),
            &rendered()
        )
        .is_err());
        assert!(inherit(
            STOCK,
            &rendered().replace(
                "sources=\"incall playback,voice call tx\"",
                "sources=\"nonexistent\""
            )
        )
        .is_err());
        assert!(inherit(
            &STOCK.replace("version=\"7.0\"", "version=\"1.0\""),
            &rendered()
        )
        .is_err());
    }
    #[test]
    fn policy_visibility_checks_the_consumers_mount_view() {
        let dir =
            std::env::temp_dir().join(format!("direct-pcm-visibility-{}", std::process::id()));
        let target = Path::new("/vendor/etc/audio_policy_configuration.xml");
        let visible = dir.join("vendor/etc/audio_policy_configuration.xml");
        fs::create_dir_all(visible.parent().unwrap()).unwrap();
        fs::write(&visible, "generated policy").unwrap();

        assert!(verify_policy_view(&dir, target, "generated policy").is_ok());
        assert!(verify_policy_view(&dir, target, "different policy")
            .unwrap_err()
            .contains("different policy"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn repeated_apply_uses_snapshot_and_new_boot_cannot_reuse_it() {
        let dir = std::env::temp_dir().join(format!("direct-pcm-snapshot-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("policy.xml");
        fs::write(&target, STOCK).unwrap();
        assert_eq!(baseline(&target, &[], "boot1", &dir).unwrap(), STOCK);
        let boot1 = snapshot_path(&target, "boot1", &dir);
        assert!(boot1.is_file());
        let candidate = dir.join("candidate.xml");
        let unrelated = dir.join("manual.xml");
        fs::write(&candidate, "candidate").unwrap();
        fs::write(&unrelated, "manual").unwrap();

        fs::write(&target, STOCK).unwrap();
        assert_eq!(baseline(&target, &[], "boot2", &dir).unwrap(), STOCK);
        let boot2 = snapshot_path(&target, "boot2", &dir);
        assert!(boot2.is_file());
        assert!(boot2.with_extension("sha256").is_file());
        assert!(!boot1.exists());
        assert!(!boot1.with_extension("sha256").exists());
        assert!(candidate.is_file());
        assert!(unrelated.is_file());

        fs::write(&target, "generated override").unwrap();
        let roots = vec![GENERATED.to_string()];
        assert_eq!(baseline(&target, &roots, "boot2", &dir).unwrap(), STOCK);
        assert!(baseline(&target, &roots, "boot1", &dir).is_err());
        assert!(baseline(&target, &["/another-module.xml".into()], "boot2", &dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
