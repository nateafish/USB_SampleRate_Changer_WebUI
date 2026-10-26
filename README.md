# USB SampleRate Changer WebUI

中文参考：[README_zh-CN.md](doc/README_zh-CN.md)

This project is a WebUI manager for [USB_SampleRate_Changer](https://github.com/yzyhk904/USB_SampleRate_Changer). The module uses a bundled Rust backend to execute the `USB_SampleRate_Changer` scripts.

> Version 1.1.1 experimentally adds an Audio Core AIDL v2/v3 backend. The original traditional XML/HIDL path and upstream submodule remain intact. Persistent AIDL application is enabled only when a vendor module-XML source matches the complete live topology.

## Repository layout

| Path | Purpose |
| --- | --- |
| `USB_SampleRate_Changer/` | `USB_SampleRate_Changer` source |
| `patches/` | Compatibility patches |
| `module/` | Module metadata and installer overlay |
| `backend/` | Rust controller |
| `webui/` | SolidJS WebUI |
| `build.sh` | Build script |
| `doc/` | Project documentation |

In the built module, all upstream scripts, templates and Extras are placed under `core/`. The module root keeps only module metadata, installation scripts, the controller and WebUI files. The Magisk/KernelSU-required `customize.sh` and `uninstall.sh` remain at the module root.

## Parameter mapping

| WebUI control | `USB_SampleRate_Changer` parameter |
| --- | --- |
| Policy template selector | `--auto`, `--offload`, `--offload-hifi-playback`, `--offload-direct`, `--offload-safer`, `--bypass-offload`, `--bypass-offload-safer`, `--legacy`, `--safe`, `--safest`, `--safest-auto`, `--usb-only` |
| Sample-rate selector | Documented 44.1–768 kHz presets, plus a custom integer in the upstream-supported 44100–768000 Hz range |
| Bit-depth selector | `16`, `24`, `32`, `float` |
| DRC switch | `--drc` |
| Force USBv2 switch | `--force-usbv2` |
| Force Bluetooth QTI switch | `--force-bluetooth-qti` |
| Bluetooth A2DP route check | Runs automatically after an audio change when A2DP was connected before the operation |
| Reset | `--reset` |

## Extras

| WebUI control | `USB_SampleRate_Changer` control |
| --- | --- |
| Bluetooth audio HAL | `change-bluetooth-hal.sh`: switch between `aosp`, `legacy`, `offload` and `sysbta` |
| AudioFlinger resampler | `change-resampling-quality.sh`: all bundled presets, or validated custom stop-band attenuation (20–242 dB), half-filter length (8–640 in steps of 8), cutoff/cheat percentage and 44.1/48/96 kHz activation threshold |
| USB transfer period | `change-usb-period.sh`: the complete 125–50000 µs range in steps of 125 µs |
| System Jitter optimization | `jitter-reducer.sh`: SELinux, thermal, Doze, CPU governor, camera, logd, I/O, virtual memory, Wi-Fi, battery and effects; I/O scheduler/tone and Wi-Fi no-restart are supported |
| Diagnostics | Filtered/full audio dumps, Bluetooth dumps, audio configuration detection and ALSA hardware parameters |

## Execution path

```text
SolidJS WebUI
  -> APatch/KernelSU root bridge (spawn, with exec fallback)
  -> usbsrctl validates the versioned command catalog
  -> render the validated shell program in memory
  -> feed it directly to the shell, entering the audioserver mount namespace when required
  -> execute the selected upstream script from the module's `core/` directory
  -> record the command summary in /data/local/tmp/usb_samplerate_changer_webui/last-command.log
```

## Build

Build requirements are Node.js, Rust, the `aarch64-linux-android` Rust target, Android NDK and `zip`. Clone the repository with its Submodule when possible:

```sh
git clone --recurse-submodules https://github.com/SivonZn/USB_SampleRate_Changer_WebUI.git
# Existing clone:
git submodule update --init --recursive
```

The build copies the current Submodule worktree into a temporary directory, verifies and applies all listed patches, builds both applications, and creates the module in the parent workspace's `output/` directory:

```sh
./build.sh
```

## Safety and compatibility

- This module modifies audio policy, audio-service properties and selected system-tuning parameters. Confirm that the device, ROM and HAL support the chosen mode.
- Experimental Audio Core AIDL v2/v3 discovery classifies runtime ports by flags, profiles and routes rather than vendor port names. Persistent AIDL writers remain vendor-family adapters and are enabled only after the backing configuration matches the live topology completely; see `doc/AIDL_V3_BACKEND.md`.
- Applying a policy or resampler configuration may briefly interrupt audio. Pay particular attention to the route-recovery warning when Bluetooth A2DP is connected.
- SELinux, thermal, Doze, governor and system-service operations may reduce security, stability or battery life and may cause overheating. Enable only options whose impact you understand.
- Actual high-sample-rate support also depends on the USB DAC, kernel, USB Audio HAL and vendor audio policy; the selector range does not mean every device supports every rate.
- Use this module at your own risk. The project is not responsible for damage caused by using it.
