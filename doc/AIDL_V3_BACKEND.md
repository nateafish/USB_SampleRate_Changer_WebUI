# Audio Core AIDL v2/v3 backend

The AIDL implementation is additive. Traditional XML and HIDL devices continue
to use the bundled `USB_SampleRate_Changer.sh` and its existing templates.

The backend does not issue version-specific Binder calls. It consumes the
framework's `media.audio_policy` topology and the vendor module XML from which
the HAL creates that topology, so the same parser and writer can cover Audio
Core AIDL v2 and v3 when all structural gates pass. VINTF is read separately to
report the interface version; it is not used as a shortcut around topology
validation.

The frozen AOSP v2 and v3 `IModule.aidl` files are byte-identical, as are the
two frozen `IConfig.aidl` files. Their common surface includes
`getAudioPorts`, `getAudioRoutes`, `setAudioPortConfig` and `setAudioPatch`.
Other package types may evolve between versions, but this backend does not
marshal any of them directly; it operates after AudioPolicy has rendered the
topology and before the vendor HAL parses its module XML.

This is a fixed policy/rate changer, matching the scope of the upstream module.
It does not implement Xiaomi's per-track source-rate-following pipeline. On
Dada, repairing a missing `sampling_rate` vendor-parameter consumer inside the
QTI HAL would require a separately validated binary adapter and is outside this
module-XML backend.

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

`aidl-preview` remains a controller-only diagnostic command. It discovers a
unique primary module configuration by comparing candidate files from ODM,
vendor, product, system_ext and system against the live topology, then emits a
validated plan without writing or mounting it. The WebUI reads the same AIDL
status model and sends Apply/Reset through the validated AIDL writer; the
traditional XML/HIDL path remains unchanged on non-AIDL devices.

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
The controller records the live Audio Core service count before the restart,
requires that service baseline plus new HAL/audioserver processes to recover,
then verifies every selected runtime PCM profile. A failure unmounts the
candidate and restarts the original HAL configuration.

The Dada Android 16 baseline declares Audio Core AIDL v2 for `IConfig/default`
and the default, USB, Bluetooth and remote-submix modules. Its init service is
also `vendor.audio-hal-aidl`, its process is `audiohalservice.qti`, and its ODM
source is the same `audio_module_config_primary.xml` schema used by the v3 QTI
baseline. The recovered Dada XML shape is covered by unit tests. This proves
offline structural compatibility; a Dada device is still required before
claiming a successful live mount/restart/profile validation for that ROM.
Persistent apply fails closed unless VINTF resolves to exactly one supported
Audio Core version, currently v2 or v3; unknown or conflicting generations
remain preview-only.

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
