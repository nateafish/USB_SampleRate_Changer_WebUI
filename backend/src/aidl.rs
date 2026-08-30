#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPolicySnapshot {
    pub config_source: String,
    pub modules: Vec<AudioModule>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioModule {
    pub name: String,
    pub output_mix_ports: Vec<AudioPort>,
    pub input_mix_ports: Vec<AudioPort>,
    pub device_ports: Vec<AudioPort>,
    pub routes: Vec<AudioRoute>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPort {
    pub name: String,
    pub flags: Vec<String>,
    pub device_type: Option<String>,
    pub connection: Option<String>,
    pub profiles: Vec<AudioProfile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioProfile {
    pub format: Option<String>,
    pub rates: Vec<u32>,
    pub dynamic_rate: bool,
    pub dynamic_format: bool,
    pub dynamic_channels: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioRoute {
    pub sink: String,
    pub sources: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    None,
    OutputMix,
    InputMix,
    Devices,
    Routes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortKind {
    OutputMix,
    InputMix,
    Device,
}

impl AudioPolicySnapshot {
    pub fn is_aidl(&self) -> bool {
        self.config_source.eq_ignore_ascii_case("AIDL HAL")
    }

    pub fn usb_output_devices(&self) -> Vec<&AudioPort> {
        self.modules
            .iter()
            .flat_map(|module| module.device_ports.iter())
            .filter(|port| is_usb_output(port))
            .collect()
    }

    pub fn connected_usb_output_rates(&self) -> Vec<u32> {
        let mut rates = self
            .usb_output_devices()
            .into_iter()
            .flat_map(AudioPort::unique_rates)
            .collect::<Vec<_>>();
        rates.sort_unstable();
        rates.dedup();
        rates
    }

    pub fn usb_reachable_mix_ports(&self) -> Vec<(&str, &AudioPort)> {
        let mut result = Vec::new();
        for module in &self.modules {
            let usb_sinks = module
                .device_ports
                .iter()
                .filter(|port| is_usb_output(port))
                .map(|port| port.name.as_str())
                .collect::<Vec<_>>();
            for route in module
                .routes
                .iter()
                .filter(|route| usb_sinks.contains(&route.sink.as_str()))
            {
                for source in &route.sources {
                    if let Some(port) = module
                        .output_mix_ports
                        .iter()
                        .find(|port| port.name == *source)
                    {
                        if !result
                            .iter()
                            .any(|(module_name, existing): &(&str, &AudioPort)| {
                                *module_name == module.name && existing.name == port.name
                            })
                        {
                            result.push((module.name.as_str(), port));
                        }
                    }
                }
            }
        }
        result
    }
}

impl AudioPort {
    pub fn unique_rates(&self) -> Vec<u32> {
        let mut rates = self
            .profiles
            .iter()
            .flat_map(|profile| profile.rates.iter().copied())
            .collect::<Vec<_>>();
        rates.sort_unstable();
        rates.dedup();
        rates
    }

    pub fn behavior(&self) -> &'static str {
        if self.has_flag("COMPRESS_OFFLOAD") {
            "compressed-offload"
        } else if self.has_flag("MMAP_NOIRQ") {
            "mmap"
        } else if self.has_flag("DEEP_BUFFER") {
            "deep-buffer-mix"
        } else if self.has_flag("PRIMARY") {
            "primary-mix"
        } else if self.has_flag("DIRECT") {
            "direct-pcm"
        } else if self.has_flag("RAW") {
            "raw"
        } else if self.profiles.is_empty()
            || self.profiles.iter().any(|profile| {
                profile.dynamic_rate || profile.dynamic_format || profile.dynamic_channels
            })
        {
            "vendor-dynamic"
        } else {
            "standard-mix"
        }
    }

    fn has_flag(&self, needle: &str) -> bool {
        self.flags.iter().any(|flag| flag.ends_with(needle))
    }
}

pub fn parse_audio_policy_dump(dump: &str) -> AudioPolicySnapshot {
    let mut snapshot = AudioPolicySnapshot::default();
    let mut in_hardware_modules = false;
    let mut current_module: Option<usize> = None;
    let mut section = Section::None;
    let mut current_port: Option<(PortKind, usize)> = None;
    let mut current_route: Option<usize> = None;

    for line in dump.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Config source:") {
            snapshot.config_source = value.trim().to_string();
            continue;
        }
        if trimmed.starts_with("Hardware modules (") {
            in_hardware_modules = true;
            continue;
        }
        if in_hardware_modules && trimmed.starts_with("Outputs (") {
            break;
        }
        if !in_hardware_modules {
            continue;
        }

        if trimmed.contains(". Handle:") {
            if let Some(name) = first_quoted(trimmed) {
                snapshot.modules.push(AudioModule {
                    name: name.to_string(),
                    ..AudioModule::default()
                });
                current_module = Some(snapshot.modules.len() - 1);
                section = Section::None;
                current_port = None;
                current_route = None;
            }
            continue;
        }

        let Some(module_index) = current_module else {
            continue;
        };
        match trimmed {
            value if value.starts_with("- Output MixPorts") => {
                section = Section::OutputMix;
                current_port = None;
                continue;
            }
            value if value.starts_with("- Input MixPorts") => {
                section = Section::InputMix;
                current_port = None;
                continue;
            }
            value if value.starts_with("- Declared devices") => {
                section = Section::Devices;
                current_port = None;
                continue;
            }
            value if value.starts_with("- Audio Routes") => {
                section = Section::Routes;
                current_port = None;
                continue;
            }
            value if value.starts_with("- Supported devices") => {
                current_port = None;
                continue;
            }
            _ => {}
        }

        match section {
            Section::OutputMix | Section::InputMix
                if is_numbered_entry(trimmed)
                    && trimmed.contains("; 0x")
                    && trimmed.contains('(') =>
            {
                if let Some(name) = first_quoted(trimmed) {
                    let port = AudioPort {
                        name: name.to_string(),
                        flags: parse_flags(trimmed),
                        ..AudioPort::default()
                    };
                    let module = &mut snapshot.modules[module_index];
                    let (kind, index) = if section == Section::OutputMix {
                        module.output_mix_ports.push(port);
                        (PortKind::OutputMix, module.output_mix_ports.len() - 1)
                    } else {
                        module.input_mix_ports.push(port);
                        (PortKind::InputMix, module.input_mix_ports.len() - 1)
                    };
                    current_port = Some((kind, index));
                }
            }
            Section::Devices
                if is_numbered_entry(trimmed) && trimmed.contains("{AUDIO_DEVICE_") =>
            {
                if let Some(name) = first_quoted(trimmed) {
                    let mut port = AudioPort {
                        name: name.to_string(),
                        ..AudioPort::default()
                    };
                    port.device_type = between(trimmed, "{", ",").map(str::to_string);
                    let module = &mut snapshot.modules[module_index];
                    module.device_ports.push(port);
                    current_port = Some((PortKind::Device, module.device_ports.len() - 1));
                }
            }
            Section::Routes if is_numbered_entry(trimmed) && trimmed.contains("; Sink:") => {
                if let Some(sink) = first_quoted(trimmed) {
                    let module = &mut snapshot.modules[module_index];
                    module.routes.push(AudioRoute {
                        sink: sink.to_string(),
                        sources: Vec::new(),
                    });
                    current_route = Some(module.routes.len() - 1);
                }
            }
            Section::Routes if trimmed.starts_with("Sources:") => {
                if let Some(route_index) = current_route {
                    snapshot.modules[module_index].routes[route_index].sources =
                        quoted_values(trimmed);
                }
            }
            _ => {
                if trimmed.starts_with("- Profiles (") {
                    if let Some(port) =
                        get_port_mut(&mut snapshot.modules[module_index], current_port)
                    {
                        port.profiles.clear();
                    }
                } else if is_numbered_entry(trimmed)
                    && (trimmed.contains("AUDIO_FORMAT_") || trimmed.contains("[dynamic"))
                {
                    if let Some(port) =
                        get_port_mut(&mut snapshot.modules[module_index], current_port)
                    {
                        port.profiles.push(AudioProfile {
                            format: trimmed
                                .split_whitespace()
                                .find(|token| token.starts_with("AUDIO_FORMAT_"))
                                .map(str::to_string),
                            dynamic_rate: trimmed.contains("[dynamic rates]"),
                            dynamic_format: trimmed.contains("[dynamic format]"),
                            dynamic_channels: trimmed.contains("[dynamic channels]"),
                            ..AudioProfile::default()
                        });
                    }
                } else if let Some(values) = trimmed.strip_prefix("sampling rates:") {
                    if let Some(port) =
                        get_port_mut(&mut snapshot.modules[module_index], current_port)
                    {
                        if port.profiles.is_empty() {
                            port.profiles.push(AudioProfile::default());
                        }
                        if let Some(profile) = port.profiles.last_mut() {
                            profile.rates = values
                                .split(|character: char| !character.is_ascii_digit())
                                .filter(|value| !value.is_empty())
                                .filter_map(|value| value.parse().ok())
                                .collect();
                        }
                    }
                }
            }
        }
    }
    snapshot
}

pub fn parse_module_xml(xml: &str) -> Result<AudioModule, String> {
    let mut module = AudioModule::default();
    let mut current: Option<(PortKind, usize)> = None;
    let mut cursor = 0;
    while let Some(start_delta) = xml[cursor..].find('<') {
        let start = cursor + start_delta;
        if xml[start..].starts_with("<!--") {
            let end = xml[start + 4..]
                .find("-->")
                .ok_or_else(|| "unterminated XML comment".to_string())?;
            cursor = start + 4 + end + 3;
            continue;
        }
        let end = find_tag_end(xml, start).ok_or_else(|| "unterminated XML tag".to_string())?;
        let tag = &xml[start + 1..end];
        cursor = end + 1;
        let trimmed = tag.trim();
        if trimmed.starts_with('?') || trimmed.starts_with('!') {
            continue;
        }
        if trimmed.starts_with("/mixPort") || trimmed.starts_with("/devicePort") {
            current = None;
            continue;
        }
        let name = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        match name {
            "module" => {
                module.name = xml_attr(trimmed, "name").unwrap_or_default();
            }
            "mixPort" => {
                let role = xml_attr(trimmed, "role").unwrap_or_default();
                let flags = xml_attr(trimmed, "flags")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let port = AudioPort {
                    name: xml_attr(trimmed, "name")
                        .ok_or_else(|| "mixPort without name".to_string())?,
                    flags,
                    ..AudioPort::default()
                };
                if role == "source" {
                    module.output_mix_ports.push(port);
                    current = Some((PortKind::OutputMix, module.output_mix_ports.len() - 1));
                } else {
                    module.input_mix_ports.push(port);
                    current = Some((PortKind::InputMix, module.input_mix_ports.len() - 1));
                }
            }
            "devicePort" => {
                let port = AudioPort {
                    name: xml_attr(trimmed, "tagName")
                        .ok_or_else(|| "devicePort without tagName".to_string())?,
                    device_type: xml_attr(trimmed, "deviceType")
                        .or_else(|| xml_attr(trimmed, "type")),
                    connection: xml_attr(trimmed, "connection"),
                    ..AudioPort::default()
                };
                module.device_ports.push(port);
                current = Some((PortKind::Device, module.device_ports.len() - 1));
            }
            "profile" => {
                if let Some(port) = get_port_mut(&mut module, current) {
                    let rates = xml_attr(trimmed, "samplingRates")
                        .unwrap_or_default()
                        .split_whitespace()
                        .filter_map(|rate| rate.parse().ok())
                        .collect();
                    port.profiles.push(AudioProfile {
                        format: xml_attr(trimmed, "pcmType")
                            .or_else(|| xml_attr(trimmed, "encoding"))
                            .or_else(|| xml_attr(trimmed, "format")),
                        rates,
                        dynamic_rate: !trimmed.contains("samplingRates="),
                        dynamic_format: !trimmed.contains("formatType=")
                            && !trimmed.contains("format="),
                        dynamic_channels: !trimmed.contains("channelLayouts=")
                            && !trimmed.contains("channelMasks="),
                    });
                }
            }
            "route" => {
                let sink =
                    xml_attr(trimmed, "sink").ok_or_else(|| "route without sink".to_string())?;
                let sources = xml_attr(trimmed, "sources")
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|source| !source.is_empty())
                    .map(str::to_string)
                    .collect();
                module.routes.push(AudioRoute { sink, sources });
            }
            _ => {}
        }
    }
    if module.name.is_empty() {
        return Err("module XML does not contain a named module".to_string());
    }
    Ok(module)
}

pub fn topology_match_score(runtime: &AudioModule, config: &AudioModule) -> (usize, usize) {
    let runtime_markers = runtime
        .output_mix_ports
        .iter()
        .map(|port| ("mix", port.name.as_str()))
        .chain(
            runtime
                .input_mix_ports
                .iter()
                .map(|port| ("mix", port.name.as_str())),
        )
        .chain(
            runtime
                .device_ports
                .iter()
                .map(|port| ("device", port.name.as_str())),
        )
        .chain(
            runtime
                .routes
                .iter()
                .map(|route| ("route", route.sink.as_str())),
        )
        .collect::<Vec<_>>();
    let matched = runtime_markers
        .iter()
        .filter(|(kind, name)| match *kind {
            "mix" => config
                .output_mix_ports
                .iter()
                .chain(config.input_mix_ports.iter())
                .any(|port| port.name == *name),
            "device" => config.device_ports.iter().any(|port| port.name == *name),
            "route" => config.routes.iter().any(|route| route.sink == *name),
            _ => false,
        })
        .count();
    (matched, runtime_markers.len())
}

pub fn rewrite_pcm_mix_port(
    xml: &str,
    port_name: &str,
    rate: u32,
    pcm_type: &str,
) -> Result<String, String> {
    let mut search = 0;
    let mut match_span = None;
    while let Some(delta) = xml[search..].find("<mixPort") {
        let start = search + delta;
        let suffix = xml.as_bytes().get(start + "<mixPort".len()).copied();
        if !suffix.is_some_and(|byte| byte.is_ascii_whitespace() || byte == b'>') {
            search = start + "<mixPort".len();
            continue;
        }
        let open_end =
            find_tag_end(xml, start).ok_or_else(|| "unterminated mixPort".to_string())?;
        let open = &xml[start + 1..open_end];
        let close_start = xml[open_end + 1..]
            .find("</mixPort>")
            .map(|delta| open_end + 1 + delta)
            .ok_or_else(|| "mixPort without closing tag".to_string())?;
        if xml_attr(open, "name").as_deref() == Some(port_name) {
            if match_span.is_some() {
                return Err(format!("mixPort {port_name} is not unique"));
            }
            match_span = Some((start, open_end + 1, close_start));
        }
        search = close_start + "</mixPort>".len();
    }
    let (start, body_start, close_start) =
        match_span.ok_or_else(|| format!("mixPort {port_name} was not found"))?;
    let body = &xml[body_start..close_start];
    let mut rewritten = String::with_capacity(body.len() + 128);
    let mut cursor = 0;
    let mut pcm_profiles = 0;
    let mut emitted: Vec<String> = Vec::new();
    while let Some(delta) = body[cursor..].find("<profile") {
        let profile_start = cursor + delta;
        let profile_end = find_tag_end(body, profile_start)
            .ok_or_else(|| format!("unterminated profile in {port_name}"))?;
        rewritten.push_str(&body[cursor..profile_start]);
        let tag = &body[profile_start..=profile_end];
        let is_pcm = xml_attr(&tag[1..tag.len() - 1], "formatType").as_deref() == Some("PCM")
            || xml_attr(&tag[1..tag.len() - 1], "format")
                .as_deref()
                .is_some_and(|format| format.starts_with("AUDIO_FORMAT_PCM_"));
        if is_pcm {
            let tag = set_xml_attr(tag, "samplingRates", &rate.to_string())?;
            let tag = set_xml_attr(&tag, "pcmType", pcm_type)?;
            if emitted.iter().any(|existing| *existing == tag) {
                trim_trailing_blank(&mut rewritten);
            } else {
                emitted.push(tag.clone());
                rewritten.push_str(&tag);
            }
            pcm_profiles += 1;
        } else {
            rewritten.push_str(tag);
        }
        cursor = profile_end + 1;
    }
    rewritten.push_str(&body[cursor..]);
    if pcm_profiles == 0 {
        if body.contains("<profile") {
            return Err(format!("mixPort {port_name} has no PCM profile"));
        }
        let indent = body
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| {
                line.chars()
                    .take_while(|character| character.is_whitespace())
                    .collect::<String>()
            })
            .unwrap_or_else(|| "    ".to_string());
        rewritten.push_str(&format!(
            "{indent}<profile samplingRates=\"{rate}\" channelLayouts=\"LAYOUT_STEREO\" formatType=\"PCM\" pcmType=\"{pcm_type}\" />\n"
        ));
    }
    let mut output = String::with_capacity(xml.len() + rewritten.len() - body.len());
    output.push_str(&xml[..body_start]);
    output.push_str(&rewritten);
    output.push_str(&xml[close_start..]);
    debug_assert_eq!(&output[..start], &xml[..start]);
    Ok(output)
}

fn trim_trailing_blank(buffer: &mut String) {
    while buffer.ends_with(' ') || buffer.ends_with('\t') {
        buffer.pop();
    }
    if buffer.ends_with('\n') {
        buffer.pop();
        if buffer.ends_with('\r') {
            buffer.pop();
        }
    }
}

fn set_xml_attr(tag: &str, key: &str, value: &str) -> Result<String, String> {
    if value.contains(['"', '\'', '<', '>', '&']) {
        return Err(format!("unsafe XML attribute value for {key}"));
    }
    let pattern = format!("{key}=\"");
    if let Some(start) = tag.find(&pattern) {
        let value_start = start + pattern.len();
        let value_end = tag[value_start..]
            .find('"')
            .map(|end| value_start + end)
            .ok_or_else(|| format!("unterminated {key} attribute"))?;
        let mut output = String::with_capacity(tag.len() + value.len());
        output.push_str(&tag[..value_start]);
        output.push_str(value);
        output.push_str(&tag[value_end..]);
        return Ok(output);
    }
    let insert = tag
        .rfind("/>")
        .or_else(|| tag.rfind('>'))
        .ok_or_else(|| "invalid XML tag".to_string())?;
    let mut output = String::with_capacity(tag.len() + key.len() + value.len() + 4);
    output.push_str(&tag[..insert]);
    if !output.ends_with(char::is_whitespace) {
        output.push(' ');
    }
    output.push_str(key);
    output.push_str("=\"");
    output.push_str(value);
    output.push('"');
    if tag[insert..].starts_with("/>") {
        output.push(' ');
    }
    output.push_str(&tag[insert..]);
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortPlan {
    pub module: String,
    pub port: String,
    pub behavior: &'static str,
    pub rate: u32,
    pub pcm_type: String,
    pub dynamic_source: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyPlan {
    pub policy: String,
    pub targets: Vec<PortPlan>,
    pub skipped: Vec<(String, &'static str, &'static str)>,
}

pub fn pcm_type_for_bit_depth(bit_depth: &str) -> Result<&'static str, String> {
    match bit_depth {
        "16" => Ok("INT_16_BIT"),
        "24" => Ok("INT_24_BIT"),
        "32" => Ok("INT_32_BIT"),
        "float" => Ok("FLOAT_32_BIT"),
        value => Err(format!("unsupported bit depth: {value}")),
    }
}

fn policy_behaviors(policy: &str) -> Result<&'static [&'static str], String> {
    match policy {
        "auto" | "safe" => Ok(&["primary-mix", "deep-buffer-mix"]),
        "safest" | "safest-auto" => Ok(&["primary-mix"]),
        "offload" => Ok(&["primary-mix", "deep-buffer-mix", "compressed-offload"]),
        "offload-direct" => Ok(&["direct-pcm", "compressed-offload"]),
        "offload-safer" => Ok(&["deep-buffer-mix", "compressed-offload"]),
        "offload-hifi-playback" | "usb" => Ok(&["vendor-dynamic"]),
        "bypass" => Ok(&["primary-mix", "deep-buffer-mix", "direct-pcm"]),
        "bypass-safer" => Ok(&["deep-buffer-mix", "direct-pcm"]),
        "legacy" => Err("legacy Bluetooth policies are not part of the AIDL backend".to_string()),
        value => Err(format!("unsupported AIDL policy: {value}")),
    }
}

pub fn plan_policy(
    snapshot: &AudioPolicySnapshot,
    policy: &str,
    rate: u32,
    bit_depth: &str,
) -> Result<PolicyPlan, String> {
    if !snapshot.is_aidl() {
        return Err("active audio policy is not AIDL".to_string());
    }
    let behaviors = policy_behaviors(policy)?;
    let pcm_type = pcm_type_for_bit_depth(bit_depth)?.to_string();
    let usb_rates = snapshot.connected_usb_output_rates();
    if !usb_rates.is_empty() && !usb_rates.contains(&rate) {
        return Err(format!(
            "connected USB output does not advertise {rate} Hz (available: {})",
            usb_rates
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    for (module, port) in snapshot.usb_reachable_mix_ports() {
        let behavior = port.behavior();
        if !behaviors.contains(&behavior) {
            continue;
        }
        let dynamic_source = port.profiles.is_empty()
            || port
                .profiles
                .iter()
                .all(|profile| profile.dynamic_rate || profile.rates.is_empty());
        let pcm_capable = dynamic_source
            || port.profiles.iter().any(|profile| {
                profile.format.as_deref().is_none_or(|format| {
                    format.contains("PCM")
                        || format.starts_with("INT_")
                        || format.starts_with("FLOAT_")
                        || format.starts_with("FIXED_")
                })
            });
        if !pcm_capable {
            skipped.push((
                format!("{module}:{}", port.name),
                behavior,
                "port only advertises compressed formats",
            ));
            continue;
        }
        targets.push(PortPlan {
            module: module.to_string(),
            port: port.name.clone(),
            behavior,
            rate,
            pcm_type: pcm_type.clone(),
            dynamic_source,
        });
    }
    if targets.is_empty() {
        return Err(format!(
            "no USB-reachable mix port matches policy {policy} at {rate} Hz"
        ));
    }
    Ok(PolicyPlan {
        policy: policy.to_string(),
        targets,
        skipped,
    })
}

pub fn apply_plan_to_module_xml(
    xml: &str,
    module_name: &str,
    plan: &PolicyPlan,
) -> Result<String, String> {
    let mut output = xml.to_string();
    for target in plan.targets.iter().filter(|target| {
        target.module == module_name || module_name == "default" || module_name == "primary"
    }) {
        output = rewrite_pcm_mix_port(&output, &target.port, target.rate, &target.pcm_type)?;
    }
    Ok(output)
}

pub fn structural_diff(before: &AudioModule, after: &AudioModule) -> Vec<String> {
    let mut differences = Vec::new();
    if before.name != after.name {
        differences.push(format!("module {} -> {}", before.name, after.name));
    }
    let names = |ports: &Vec<AudioPort>| {
        ports
            .iter()
            .map(|port| port.name.clone())
            .collect::<Vec<_>>()
    };
    if names(&before.output_mix_ports) != names(&after.output_mix_ports) {
        differences.push("output mix port set changed".to_string());
    }
    if names(&before.input_mix_ports) != names(&after.input_mix_ports) {
        differences.push("input mix port set changed".to_string());
    }
    if names(&before.device_ports) != names(&after.device_ports) {
        differences.push("device port set changed".to_string());
    }
    if before.routes != after.routes {
        differences.push("route table changed".to_string());
    }
    differences
}

pub fn profile_diff(before: &AudioModule, after: &AudioModule) -> Vec<String> {
    let mut differences = Vec::new();
    for old in &before.output_mix_ports {
        let Some(new) = after
            .output_mix_ports
            .iter()
            .find(|port| port.name == old.name)
        else {
            continue;
        };
        if old.profiles != new.profiles {
            differences.push(format!(
                "{}: {} -> {}",
                old.name,
                summarize_profiles(old),
                summarize_profiles(new)
            ));
        }
    }
    differences
}

fn summarize_profiles(port: &AudioPort) -> String {
    if port.profiles.is_empty() {
        return "dynamic".to_string();
    }
    port.profiles
        .iter()
        .map(|profile| {
            let rates = if profile.rates.is_empty() {
                "dynamic".to_string()
            } else {
                profile
                    .rates
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("/")
            };
            match profile.format.as_deref() {
                Some(format) => format!("{format}@{rates}"),
                None => rates,
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_usb_output(port: &AudioPort) -> bool {
    let output = port
        .device_type
        .as_deref()
        .is_some_and(|kind| kind.contains("OUT_") || kind.starts_with("AUDIO_DEVICE_OUT"));
    output
        && (port
            .device_type
            .as_deref()
            .is_some_and(|kind| kind.contains("USB"))
            || port.connection.as_deref() == Some("usb"))
}

fn find_tag_end(xml: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in xml[start + 1..].char_indices() {
        match character {
            '"' | '\'' if quote == Some(character) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(character),
            '>' if quote.is_none() => return Some(start + 1 + offset),
            _ => {}
        }
    }
    None
}

fn xml_attr(tag: &str, key: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut cursor = bytes.iter().position(|byte| byte.is_ascii_whitespace())?;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && bytes[cursor] != b'='
            && bytes[cursor] != b'/'
        {
            cursor += 1;
        }
        let name = &tag[name_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            cursor += 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = *bytes.get(cursor)?;
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        cursor += 1;
        let value_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != quote {
            cursor += 1;
        }
        if name == key {
            return Some(tag[value_start..cursor].to_string());
        }
        cursor += 1;
    }
    None
}

fn get_port_mut(
    module: &mut AudioModule,
    current: Option<(PortKind, usize)>,
) -> Option<&mut AudioPort> {
    match current? {
        (PortKind::OutputMix, index) => module.output_mix_ports.get_mut(index),
        (PortKind::InputMix, index) => module.input_mix_ports.get_mut(index),
        (PortKind::Device, index) => module.device_ports.get_mut(index),
    }
}

fn is_numbered_entry(value: &str) -> bool {
    let Some((prefix, _)) = value.split_once('.') else {
        return false;
    };
    !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit())
}

fn first_quoted(value: &str) -> Option<&str> {
    between(value, "\"", "\"")
}

fn quoted_values(mut value: &str) -> Vec<String> {
    let mut output = Vec::new();
    while let Some(start) = value.find('"') {
        let rest = &value[start + 1..];
        let Some(end) = rest.find('"') else {
            break;
        };
        output.push(rest[..end].to_string());
        value = &rest[end + 1..];
    }
    output
}

fn between<'a>(value: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_index = value.find(start)? + start.len();
    let remaining = &value[start_index..];
    let end_index = remaining.find(end)?;
    Some(remaining[..end_index].trim())
}

fn parse_flags(value: &str) -> Vec<String> {
    let Some(flags) = between(value, "(", ")") else {
        return Vec::new();
    };
    flags
        .split('|')
        .map(str::trim)
        .filter(|flag| {
            flag.starts_with("AUDIO_OUTPUT_FLAG_") || flag.starts_with("AUDIO_INPUT_FLAG_")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
 Config source: AIDL HAL
 Hardware modules (1):
  1. Handle: 10; "primary"
   - Output MixPorts (4):
      1. "low_latency_out"; 0x0006 (AUDIO_OUTPUT_FLAG_PRIMARY|AUDIO_OUTPUT_FLAG_FAST)
       - Profiles (1):
          1. ""; AUDIO_FORMAT_PCM_24_BIT_PACKED (0x6)
             sampling rates: 48000
      2. "deep_buffer_out"; 0x0008 (AUDIO_OUTPUT_FLAG_DEEP_BUFFER)
       - Profiles (1):
          1. ""; AUDIO_FORMAT_PCM_32_BIT (0x3)
             sampling rates: 48000
      3. "direct_pcm_out"; 0x0001 (AUDIO_OUTPUT_FLAG_DIRECT)
       - Profiles (1):
          1. ""; AUDIO_FORMAT_PCM_32_BIT (0x3)
             sampling rates: 44100, 48000, 96000
      4. "vendor_rate_port"; 0x0000 (AUDIO_OUTPUT_FLAG_NONE)
       - Profiles (1):
          1. ""; [dynamic format][dynamic channels][dynamic rates]; AUDIO_FORMAT_DEFAULT (0x0)
   - Declared devices (2):
    1. "speaker"; {AUDIO_DEVICE_OUT_SPEAKER, @:}
    2. "usb_sink"; {AUDIO_DEVICE_OUT_USB_DEVICE, @:}
   - Audio Routes (2):
      1. Mix; Sink: "speaker"
         Sources: "low_latency_out", "deep_buffer_out"
      2. Mix; Sink: "usb_sink"
         Sources: "low_latency_out", "deep_buffer_out", "direct_pcm_out", "vendor_rate_port"
 Outputs (0):
"#;

    #[test]
    fn parses_aidl_topology_without_port_name_assumptions() {
        let snapshot = parse_audio_policy_dump(FIXTURE);
        assert!(snapshot.is_aidl());
        assert_eq!(snapshot.modules.len(), 1);
        assert_eq!(snapshot.usb_output_devices()[0].name, "usb_sink");
        let reachable = snapshot.usb_reachable_mix_ports();
        assert_eq!(reachable.len(), 4);
        assert_eq!(reachable[0].1.behavior(), "primary-mix");
        assert_eq!(reachable[1].1.behavior(), "deep-buffer-mix");
        assert_eq!(reachable[2].1.behavior(), "direct-pcm");
        assert_eq!(reachable[3].1.behavior(), "vendor-dynamic");
    }

    #[test]
    fn does_not_treat_traditional_config_as_aidl() {
        let snapshot =
            parse_audio_policy_dump(" Config source: /vendor/etc/audio_policy_configuration.xml\n");
        assert!(!snapshot.is_aidl());
    }

    #[test]
    fn parses_qti_module_xml_by_semantics() {
        let xml = r#"<modules><module name="default"><mixPorts>
          <mixPort name="music" role="source" flags="FAST PRIMARY">
            <profile samplingRates="44100 48000" channelLayouts="LAYOUT_STEREO" formatType="PCM" pcmType="INT_24_BIT"/>
          </mixPort>
          <mixPort name="manufacturer_dynamic" role="source"></mixPort>
        </mixPorts><devicePorts>
          <devicePort tagName="external converter" role="sink" deviceType="OUT_DEVICE" connection="usb"/>
        </devicePorts><routes>
          <route type="mix" sink="external converter" sources="music,manufacturer_dynamic"/>
        </routes></module></modules>"#;
        let module = parse_module_xml(xml).unwrap();
        assert_eq!(module.name, "default");
        assert_eq!(module.output_mix_ports[0].behavior(), "primary-mix");
        assert_eq!(module.output_mix_ports[1].behavior(), "vendor-dynamic");
        assert!(is_usb_output(&module.device_ports[0]));
        assert_eq!(module.routes[0].sources.len(), 2);
    }

    #[test]
    fn plans_policies_by_behavior_not_by_port_name() {
        let snapshot = parse_audio_policy_dump(FIXTURE);
        let plan = plan_policy(&snapshot, "bypass", 48_000, "32").unwrap();
        let behaviors = plan
            .targets
            .iter()
            .map(|target| target.behavior)
            .collect::<Vec<_>>();
        assert_eq!(behaviors, ["primary-mix", "deep-buffer-mix", "direct-pcm"]);
        assert!(plan
            .targets
            .iter()
            .all(|target| target.pcm_type == "INT_32_BIT"));

        let vendor = plan_policy(&snapshot, "offload-hifi-playback", 96_000, "32").unwrap();
        assert_eq!(vendor.targets.len(), 1);
        assert_eq!(vendor.targets[0].behavior, "vendor-dynamic");
        assert!(vendor.targets[0].dynamic_source);
    }

    #[test]
    fn rewrites_static_mix_ports_beyond_their_current_rate() {
        let snapshot = parse_audio_policy_dump(FIXTURE);
        let plan = plan_policy(&snapshot, "bypass", 96_000, "32").unwrap();
        assert_eq!(plan.targets.len(), 3);
        assert!(plan
            .targets
            .iter()
            .any(|target| target.behavior == "primary-mix"));
        assert!(plan_policy(&snapshot, "safest", 192_000, "32").is_ok());
        assert!(plan_policy(&snapshot, "legacy", 48_000, "32").is_err());
    }

    #[test]
    fn limits_plans_to_connected_usb_device_rates() {
        let mut snapshot = parse_audio_policy_dump(FIXTURE);
        snapshot.modules[0].device_ports.push(AudioPort {
            name: "connected converter".to_string(),
            device_type: Some("AUDIO_DEVICE_OUT_USB_HEADSET".to_string()),
            profiles: vec![AudioProfile {
                format: Some("AUDIO_FORMAT_PCM_32_BIT".to_string()),
                rates: vec![44_100, 48_000],
                ..AudioProfile::default()
            }],
            ..AudioPort::default()
        });
        assert_eq!(snapshot.connected_usb_output_rates(), vec![44_100, 48_000]);
        assert!(plan_policy(&snapshot, "auto", 44_100, "32").is_ok());
        assert!(plan_policy(&snapshot, "auto", 96_000, "32").is_err());
    }

    #[test]
    fn skips_compressed_only_ports_instead_of_failing() {
        let dump = r#"
 Config source: AIDL HAL
 Hardware modules (1):
  1. Handle: 10; "primary"
   - Output MixPorts (2):
      1. "offload_out"; 0x0011 (AUDIO_OUTPUT_FLAG_DIRECT|AUDIO_OUTPUT_FLAG_COMPRESS_OFFLOAD)
       - Profiles (1):
          1. ""; AUDIO_FORMAT_MP3 (0x1000000)
             sampling rates: 44100, 48000
      2. "direct_out"; 0x0001 (AUDIO_OUTPUT_FLAG_DIRECT)
       - Profiles (1):
          1. ""; AUDIO_FORMAT_PCM_16_BIT (0x1)
             sampling rates: 44100, 48000
   - Declared devices (1):
    1. "dock"; {AUDIO_DEVICE_OUT_USB_HEADSET, @:}
   - Audio Routes (1):
      1. Mix; Sink: "dock"
         Sources: "offload_out", "direct_out"
 Outputs (0):
"#;
        let snapshot = parse_audio_policy_dump(dump);
        let plan = plan_policy(&snapshot, "offload-direct", 48_000, "24").unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].port, "direct_out");
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].2.contains("compressed"));
    }

    #[test]
    fn rewrites_only_selected_pcm_mix_port() {
        let xml = r#"<modules><module name="default"><mixPorts>
  <mixPort name="normal" role="source"><profile samplingRates="48000" channelLayouts="LAYOUT_STEREO" formatType="PCM" pcmType="INT_24_BIT" /></mixPort>
  <mixPort name="direct" role="source"><profile samplingRates="44100 48000" channelLayouts="LAYOUT_STEREO" formatType="PCM" pcmType="INT_16_BIT" /></mixPort>
</mixPorts></module></modules>"#;
        let rewritten = rewrite_pcm_mix_port(xml, "direct", 96_000, "INT_32_BIT").unwrap();
        assert!(
            rewritten.contains("name=\"normal\" role=\"source\"><profile samplingRates=\"48000\"")
        );
        assert!(
            rewritten.contains("name=\"direct\" role=\"source\"><profile samplingRates=\"96000\"")
        );
        assert!(rewritten.contains("pcmType=\"INT_32_BIT\""));
    }

    #[test]
    fn collapses_profiles_that_become_identical() {
        let xml = concat!(
            "<modules><module name=\"default\"><mixPorts>\n",
            "  <mixPort name=\"direct\" role=\"source\">\n",
            "    <profile samplingRates=\"44100 48000\" formatType=\"PCM\" pcmType=\"INT_16_BIT\" />\n",
            "    <profile samplingRates=\"44100 48000\" formatType=\"PCM\" pcmType=\"INT_24_BIT\" />\n",
            "    <profile samplingRates=\"44100 48000\" formatType=\"PCM\" pcmType=\"INT_32_BIT\" />\n",
            "  </mixPort>\n",
            "</mixPorts></module></modules>"
        );
        let rewritten = rewrite_pcm_mix_port(xml, "direct", 96_000, "INT_32_BIT").unwrap();
        assert_eq!(rewritten.matches("<profile").count(), 1);
        let module = parse_module_xml(&rewritten).unwrap();
        assert_eq!(module.output_mix_ports[0].profiles.len(), 1);
        assert_eq!(module.output_mix_ports[0].profiles[0].rates, vec![96_000]);
    }

    #[test]
    fn can_materialize_an_empty_dynamic_mix_port() {
        let xml = "<modules><module name=\"default\"><mixPorts><mixPort name=\"vendor\" role=\"source\">\n</mixPort></mixPorts></module></modules>";
        let rewritten = rewrite_pcm_mix_port(xml, "vendor", 44_100, "INT_24_BIT").unwrap();
        assert!(rewritten.contains("samplingRates=\"44100\""));
        assert!(rewritten.contains("formatType=\"PCM\""));
    }
}
