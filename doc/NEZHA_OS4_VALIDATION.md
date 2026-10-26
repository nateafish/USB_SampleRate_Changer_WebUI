# Experimental AIDL validation on Nezha OS4

Tested on 2026-10-26 with module version 1.2.7, based on upstream 1.2.6
(`5397169`). This is a policy/profile loading test, not a USB DAC output test.

## Device and initial state

| Item | Observed value |
| --- | --- |
| Device | Xiaomi 17 Ultra, 2512BPNDAC / nezha |
| System | OS4.0.5.0.XPACNXM, Android 17 / API 37 |
| Security patch | 2026-10-01 |
| Audio Core | AIDL v3, confirmed from VINTF |
| Services | IConfig/default; IModule/default, usb, bluetooth, r_submix |
| Init service / process | vendor.audio-hal-aidl / audiohalservice.qti |
| Active configuration | /odm/etc/audio/audio_module_config_primary.xml |
| Root environment | KernelSU, SELinux enforcing |
| Mount namespace | init, ADB shell, HAL and audioserver shared the global namespace |
| Initial audio state | No previous audio modules, controller state or audio configuration bind mount |
| External USB DAC | Not connected; USB was used for ADB |

The ODM XML matched all 83 live topology markers. The vendor XML did not match
completely and was excluded. The ODM XML was byte-identical to the previously
captured Nezha baseline. Device type matching accounts for the AIDL
type/connection representation and its framework enum translation; it does
not treat generic OUT_DEVICE as an unrestricted wildcard.

## Completed on-device checks

1. Installed the packaged module through KernelSU and rebooted to activate it.
   The module contains `skip_mount`; installing it left the audio XML unchanged
   and automatic reapplication disabled. No additional overlay module or HAL
   binary patch was installed.
2. Ran Reset before any Apply. It succeeded without restarting audio services.
3. Previewed `offload-direct` at 96000 Hz / 32-bit PCM. The selected target was
   `direct_pcm_out`, identified by its DIRECT flag and USB route reachability.
4. Applied that configuration with a file bind mount. The HAL and audioserver
   received new PIDs and all five Audio Core service registrations recovered.
5. Read back the live AudioPolicy profile: `direct_pcm_out` exposed
   `AUDIO_FORMAT_PCM_32_BIT` at 96000 Hz. Primary and deep-buffer mix ports
   remained at 48000 Hz; the vendor dynamic port remained dynamic.
6. Reset the module. The bind mount disappeared and the original Direct PCM
   profiles returned. The ODM file SHA-256 again matched the initial baseline:
   `369b5a595837d78ee6d7f1ad7042129421d9cdc3ea27b1c229e8476a54c9f151`.

The phone was left with module 1.2.7 installed, `policy_configured=0`,
`auto_reapply=0`, `aidl_mount_active=0` and healthy controller state.

## Host verification

- 101 Rust tests and 49 WebUI tests passed; TypeScript and production builds passed.
- Android arm64 release build and module packaging passed; the upstream script
  submodule remains at `8c98d6d` without local modifications.
- Offline 96000 Hz / 32-bit candidates for auto, bypass, offload, offload-direct,
  vendor-dynamic and safest retained the complete 83/83 topology. These are
  candidate validation results, not live playback verification for every mode.

## Remaining gaps

- Only one QTI AIDL v3 device/build has been validated live. AIDL v2 coverage
  is currently based on offline Dada XML/VINTF shapes and unit tests.
- External DAC negotiation, actual ALSA output rates and audible playback
  were not tested in this session. A loaded policy rate does not prove a
  hardware output rate or bit-perfect playback.
- Vendor dynamic ports and other policy modes still need live apply/playback
  checks. Primary and deep-buffer ports can also feed non-USB devices; the
  adapter does not split shared mix ports to isolate the speaker path.
- Legacy Bluetooth policy, traditional XML switches, and mainline dynamic
  template inheritance are not implemented by the AIDL writer.
- Failure rollback, boot reapplication, OTA changes and uninstall recovery
  need additional live tests. Apply and Reset succeeded in the normal case.
- Other vendor configuration formats and separate HAL mount namespaces need
  their own adapters; an unsupported topology must remain read-only.
- This module does not implement per-track source sample-rate following.
