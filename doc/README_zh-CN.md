# USB SampleRate Changer WebUI

English reference：[README.md](../README.md)


本项目为 [USB_SampleRate_Changer](https://github.com/yzyhk904/USB_SampleRate_Changer)的 WebUI 管理器。模块通过内置的 Rust 二进制后端用于执行 `USB_SampleRate_Changer` 的脚本。

> 1.0.6 实验性加入 Audio Core AIDL v2/v3 后端。原有传统 XML/HIDL 路径及上游子模块保持不变；只有厂商 module XML 与实时拓扑完整匹配时，才允许持久应用 AIDL 配置。

## 仓库结构

| 路径 | 用途 |
| --- | --- |
| `USB_SampleRate_Changer/` | `USB_SampleRate_Changer` 源码 |
| `patches/` | 兼容性补丁 |
| `module/` | 模块元数据和安装覆盖文件 |
| `backend/` | Rust 控制器 |
| `webui/` | SolidJS WebUI |
| `build.sh` | 编译脚本 |
| `doc/` | 项目文档 |

构建出的模块会将上游脚本、模板和 Extras 全部放在模块根目录的 `core/` 中；根目录仅保留模块元数据、安装脚本、控制器和 WebUI 文件。Magisk/KernelSU 要求的 `customize.sh` 与 `uninstall.sh` 仍位于模块根目录。

## 参数对应关系

| WebUI 控件 | `USB_SampleRate_Changer` 参数 |
| --- | --- |
| 策略模板选择器 | `--auto`、`--offload`、`--offload-hifi-playback`、`--offload-direct`、`--offload-safer`、`--bypass-offload`、`--bypass-offload-safer`、`--legacy`、`--safe`、`--safest`、`--safest-auto`、`--usb-only` |
| 采样率选择器 | 文档列出的 44.1–768 kHz 预设，以及上游支持范围内的 44100–768000 Hz 自定义整数 |
| 位深选择器 | `16`、`24`、`32`、`float` |
| DRC 开关 | `--drc` |
| 强制 USBv2 开关 | `--force-usbv2` |
| 强制 Bluetooth QTI 开关 | `--force-bluetooth-qti` |
| Bluetooth A2DP 路由检查 | 当执行前检测到 A2DP 已连接时，在音频修改后自动运行 |
| 重置 | `--reset` |

## Extras 功能

| WebUI 控件 | `USB_SampleRate_Changer` 控制项 |
| --- | --- |
| 蓝牙音频 HAL | `change-bluetooth-hal.sh`：`aosp`、`legacy`、`offload`、`sysbta` 切换 |
| AudioFlinger 重采样器 | `change-resampling-quality.sh`：所有内置预设，或经过校验的自定义阻带衰减（20–242 dB）、半滤波器长度（8–640，步进 8）、cutoff/cheat 百分比和 44.1/48/96 kHz 生效阈值 |
| USB 传输周期 | `change-usb-period.sh`：完整的 125–50000 μs 范围（步进 125 μs） |
| 系统 Jitter 优化 | `jitter-reducer.sh`：SELinux、温控、Doze、调频 governor、相机、logd、I/O、虚拟内存、Wi-Fi、电池和音效；支持 I/O scheduler/tone 以及 Wi-Fi no-restart |
| 诊断 | 过滤/完整音频 dump、Bluetooth dump、音频配置探测和 ALSA 硬件参数 |

## 执行路径

```text
SolidJS WebUI
  -> APatch/KernelSU root bridge（优先 spawn，exec 作为回退）
  -> usbsrctl 校验版本化命令目录
  -> 在内存中生成已校验的 shell 程序
  -> 直接送入 shell，并在需要时进入 audioserver 的 mount namespace
  -> 执行模块 `core/` 中选定的上游脚本
  -> 将命令摘要记录到 /data/local/tmp/usb_samplerate_changer_webui/last-command.log
```

## 构建

构建需要 Node.js、Rust、`aarch64-linux-android` Rust target、Android NDK 和 `zip`。克隆仓库时建议同时初始化子模块：

```sh
git clone --recurse-submodules https://github.com/SivonZn/USB_SampleRate_Changer_WebUI.git
# 已有仓库：
git submodule update --init --recursive
```

构建流程会将当前子模块工作区复制到临时目录，校验并应用所有列出的补丁，编译两个应用，最后在上级工作区的 `output/` 目录生成模块：

```sh
./build.sh
```

## 安全与兼容性提示

- 本模块会修改音频策略、音频服务属性以及部分系统调优参数。请确认设备、ROM 和 HAL 支持所选模式。
- 实验性 Audio Core AIDL v2/v3 解析按 flags、动态 profile 与 route 分类端口，不依赖厂商端口名；持久写回仍属于厂商家族适配器，详见 `AIDL_V3_BACKEND.md`。
- 应用策略或重采样配置可能短暂中断音频；Bluetooth A2DP 已连接时尤其需要注意路由恢复提示。
- SELinux、温控、Doze、调频和系统服务相关操作可能降低安全性、稳定性或续航，并可能导致设备过热。请只启用自己理解其影响的选项。
- 高采样率是否真正可用还取决于 USB DAC、内核、USB Audio HAL 和厂商音频策略；选择器中的范围不代表每台设备都支持全部采样率。
- 本项目不对尝试使用本模块可能造成的设备损坏负责，是否使用由用户自行决定。
