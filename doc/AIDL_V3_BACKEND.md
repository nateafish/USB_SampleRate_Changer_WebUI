# AIDL v3 audio backend

The AIDL implementation is additive. Traditional XML and HIDL devices continue
to use the bundled `USB_SampleRate_Changer.sh` and its existing templates.

## Layers

1. Runtime discovery parses the framework view of the AIDL `IModule` topology:
   modules, output/input mix ports, device ports, profiles and routes.
2. Ports are classified by direction, flags, profile behavior and reachability.
   Names such as `hifi_playback` are diagnostic labels, not compatibility keys.
3. A persistent writer is selected only after its source configuration matches
   the runtime module topology completely. A QTI module XML writer must not be
   used for another vendor's AIDL implementation.

The controller currently exposes these diagnostic commands:

```text
usbsrctl aidl-inspect
usbsrctl aidl-inspect-stdin
usbsrctl aidl-preview POLICY RATE DEPTH
usbsrctl aidl-match-stdin XML
usbsrctl aidl-rewrite-validate-stdin XML PORT RATE DEPTH
usbsrctl aidl-plan-stdin XML POLICY RATE DEPTH [OUT]
```

The stdin variants make the parser testable on the host with a captured
`dumpsys media.audio_policy` without modifying a device.

`aidl-preview` discovers a unique primary module configuration by comparing
candidate files from ODM, vendor, product, system_ext and system against the
live topology. It emits only a validated plan and never writes or mounts a
candidate. The WebUI uses this command on AIDL devices and disables the
traditional apply/reset path.

## Runtime model

USB output devices are recognized by the AIDL device description translated by
AudioPolicy, not by tag names. A mix port is considered USB-reachable only when
an AudioRoute connects it to such a device port. Standard behavior classes are
derived from flags:

- primary mix;
- deep-buffer mix;
- direct PCM;
- compressed offload;
- MMAP;
- raw;
- vendor dynamic;
- standard mix.

## Persistence boundary

Audio Core AIDL does not expose setters for the module's port/profile/route
topology. `setAudioPortConfig` and `setAudioPatch` create runtime state which is
owned by AudioPolicy and can be reset. Persistent changes therefore use a
writer for the configuration source from which the HAL reconstructs its AIDL
objects.

For the tested QTI implementation the source is an
`audio_module_config_primary.xml`. Before any rewrite, the writer must compare
the source model with the live module model. The Nezha Android 17 ODM file
matched all 83 runtime topology markers. Rewritten candidates for primary,
deep-buffer, direct-PCM and vendor-dynamic ports retained the same 83/83
topology in offline validation.

Application uses a validated bind mount and restarts `vendor.audio-hal-aidl`.
The controller requires all five core AIDL services and new HAL/audioserver
processes to recover, then verifies every selected runtime PCM profile. A
failure unmounts the candidate and restarts the original HAL configuration.

## Policy mapping

Policies select port behaviors, never port names:

| Policy | Behaviors |
| --- | --- |
| `auto`, `safe` | primary mix, deep-buffer mix |
| `safest`, `safest-auto` | primary mix |
| `offload` | primary mix, deep-buffer mix, compressed offload |
| `offload-safer` | deep-buffer mix, compressed offload |
| `offload-direct` | direct PCM, compressed offload |
| `bypass` | primary mix, deep-buffer mix, direct PCM |
| `bypass-safer` | deep-buffer mix, direct PCM |
| `offload-hifi-playback`, `usb` | vendor dynamic |
| `legacy` | rejected; it targets legacy Bluetooth HALs |

Static mix-port rates describe the currently loaded policy and are rewrite
targets, not hardware capability limits. When a USB DAC is connected, planning
instead limits rates to the dynamic profiles advertised by that external
device. Ports that only expose compressed formats are reported as skipped
instead of being rewritten.

## Candidate generation

`aidl-plan-stdin` builds a candidate configuration and refuses to emit it unless
every check passes:

- the source configuration matches the live module topology completely;
- the candidate still matches that topology;
- no module, port or route is added, renamed or removed;
- rewritten PCM profiles that become identical are collapsed into one.

The offline command prints the selected targets, skipped ports and profile
changes, then reports `apply_enabled=0` because that command never mutates a
device. The normal controller `apply` command uses the same four validation
gates before mounting. It stores the original and candidate under the module
state directory; boot restore refuses to proceed after an OTA changes the
underlying source XML. Traditional XML/HIDL devices still use the upstream
script unchanged.
