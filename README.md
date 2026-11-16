# RMK Configuration for Cornix Keyboard

This repository contains an unofficial [RMK](https://rmk.rs/) configuration for
the Cornix keyboard by Jezail Funder. It aims to help users to customize their
own RMK firmware for Cornix, not to replicate the official firmware.

# Features

- It supports all keys and rotary encoders.
- It supports Vial.
- It supports the onboard RGB status indicators for battery and connection
  status.
- Its Vial layout is roughlly compatible with the official firmware, so you can
  load your existing Vial layout (`.vil` file) without much modification.
  Macros, combos, tap dances, key maps for rotary encoders, and some other
  things may lost or be messed up, so you may still need to reconfigure them.

# Notes

- Full RGB lighting effects are not supported. The onboard LEDs are used only as
  short status indicators.
- BLE is configured for balanced daily use: 2M PHY, +8 dBm TX power, and split
  central sleep after 2 minutes of inactivity.
- Status LEDs use an adaptive refresh: they run smooth, gamma-corrected
  breathing with soft fade in/out while active, and drop to a slow idle tick
  once an effect becomes static or turns off, so idle wakeups stay low.
- Status LEDs are power-limited. Connection/profile events show for 3 seconds,
  advertising/disconnected breathing stops after 60 seconds, and low battery
  reminders pulse briefly every 5 minutes.

# Status LEDs

Each half has two independently controlled WS2812 LEDs. Effects are evaluated
in priority order from top to bottom; when a temporary effect expires, the next
active effect is restored automatically.

## Central (left) half

| LED | Priority | Behavior |
| --- | --- | --- |
| Inner (right) | USB charging | Breathes green while charging; shows solid green for 3 seconds after reaching 95%, then turns off |
| Inner (right) | Low battery | Double-pulses coral for 5 seconds at 20% or below, repeating about every 5 minutes |
| Inner (right) | Split connection | Breathes blue for up to 60 seconds while disconnected; shows solid blue for 3 seconds after connecting |
| Outer (left) | BLE connection/profile | Shows the active profile color for 3 seconds after connecting or switching profiles |
| Outer (left) | BLE advertising | Breathes the active profile color for up to 60 seconds |
| Outer (left) | Caps Lock | Stays amber while Caps Lock is active |

BLE profile colors are deep gem tones: amethyst (profile 1), emerald (profile
2), sapphire (profile 3), garnet (profile 4), and diamond (profile 5).

## Peripheral (right) half

| LED | Priority | Behavior |
| --- | --- | --- |
| Inner | USB charging | Breathes green while charging; shows solid green for 3 seconds after reaching 95%, then turns off |
| Inner | Low battery | Double-pulses coral for 5 seconds at 20% or below, repeating about every 5 minutes |
| Outer | Split connection | Breathes blue for up to 60 seconds while disconnected; shows solid blue for 3 seconds after connecting |

Both LEDs fade smoothly between states and turn off while the keyboard is
sleeping. Animated effects and fades update every 33 ms; settled solid colors
and off states use a 1-second idle tick. State-change events still trigger a
prompt update. Battery voltage is sampled every 5 minutes where an ADC is
available, and the LED power rail is shut down after both LEDs fade fully to
black.

# Usage

## RMK 0.9.0 升级说明

本项目固定使用 RMK `0.9.0`，继续使用 Vial、Adafruit Bootloader 和原有引脚、
电池分压校准。左右半电池 ADC 统一由 `keyboard.toml` 配置，采样周期保持五分钟。
状态灯迁移到新版处理器接口，保留渐变、自适应刷新及休眠停灯行为。

刷机前请导出 Vial `.vil` 配置，并保存旧版左右半 UF2。RMK 的构建哈希包含构建时间，
哈希变化会使存储重新初始化，不能依靠 `clear_storage = false` 保留配置。
左右半必须使用同一套新版固件；升级后删除电脑／手机上的旧蓝牙配对并重新配对，
再恢复并检查按键、旋钮、宏、组合键及 Tap Dance 配置。回退旧固件同样可能需要恢复配置。

本地构建除 Rust `1.96.0`、`flip-link`、`cargo-make` 和 UF2 转换工具外，
还需要完整的 ARM GNU 裸机工具链（`arm-none-eabi-gcc`、`arm-none-eabi-ar` 及 newlib 头文件）。
新版蓝牙加密依赖会编译 C／汇编；请将工具链的 `bin` 目录加入 `PATH`。

`vendor/rmk` 和 `vendor/rmk-macro` 基于上游 `rmk-v0.9.0` 标签，保留以下本地调整：

- 将重启／Bootloader 操作转发给按键所在半边。
- 断线后等待事件队列空位释放分体按键，清理完成再重连；服务发现和分体写入均设有超时。
- 切换蓝牙槽位前清理按键、组合键临时层、修饰键和鼠标状态，忽略旧按键的后续松开；保留主动切换的常驻层。
- 双侧电量通知等待新的按键，避免后台电量变化直接唤醒主机；保留上游缓存读取。
- 电池 ADC 默认每五分钟采样一次。

固件构建与检查：

```sh
cargo fmt --check
cargo clippy --release --bins --locked -- -D warnings
cargo build --release --locked
cargo make uf2
```

库测试使用 nextest 隔离全局模拟时钟，并使用独立夹具配置。安装
`cargo-nextest 0.9.152` 后，在项目根目录运行（目标可替换为本机宿主平台）：

```sh
KEYBOARD_TOML_PATH="$PWD/tests/rmk.toml" cargo nextest run \
  --manifest-path vendor/rmk/Cargo.toml --target aarch64-apple-darwin \
  --no-default-features --features std,log,split,_ble,vial,storage --lib
```

实机验收应覆盖双侧按键及旋钮、Vial 保存与恢复、左右半电量、灯效、蓝牙槽位切换、
单侧重启、断线重连、电脑休眠时是否被意外唤醒，以及键盘休眠后的首次输入。

参考：[官方迁移指南](https://rmk.rs/docs/migration/v08_v09)。

## 构建与刷机

1. Make any changes you want for the firmware.

2. Build the firmware. Execute in the repository root:
   ```sh
   cargo make uf2
   ```
   This will generate two `.uf2` files in the repository root. Make sure you
   have the pinned Rust toolchain and `cargo-make` installed. `cargo build
   --release` only builds the ELF binaries under `target/`.

   Otherwise, fork this repository, go to GitHub Actions tab, tap *Build RMK
   firmware*, and download the artifacts when the build is done.

3. Flash the two `.uf2` files to the left and right halves of the keyboard
   respectively. You may need to delete Bluetooth pairing on your computer first
   and re-pair after flashing.
