# Cornix 键盘 RMK 固件

[English](README.en.md)

本仓库为 Jezail Funder 的 Cornix 键盘提供非官方 [RMK](https://rmk.rs/) 固件配置，
供用户定制使用。项目固定使用 RMK `0.9.0`，采用 nRF52840 和 Adafruit Bootloader。

## 功能与兼容性

- 支持全部按键、双侧旋钮和 Vial 配置。
- 支持双侧电量读取，以及板载 WS2812 电量、连接和 Caps Lock 状态提示。
- Vial 布局与官方固件大致兼容，可导入已有 `.vil` 文件；宏、组合键、Tap Dance
  和旋钮映射等仍需逐项检查，必要时重新配置。
- 板载 RGB 仅用于状态提示，不提供完整 RGB 灯效。

## 优化内容

### 连接稳定性与输入恢复

- 分体服务发现、消息写入设有超时；右半发送消息最多等待 5 秒，发送失败或超时后
  退出会话并主动断开连接，进入重连流程。
- 断线后等待事件队列空位，释放分体按键，清理完成后再重连。
- 左半休眠且仍有主机连接时，会为离线的右半继续尝试重连。单次连接或等待广播的窗口为
  1 秒，尝试结束后间隔 5 秒。无主机连接时，左半等待唤醒后再重连。
- 切换蓝牙槽位前清理按键、组合键临时层、修饰键和鼠标状态，忽略旧按键的后续松开，
  保留主动切换的常驻层。
- 重启／Bootloader 操作转发给触发按键所在的半边。

### 电量与省电

- BLE 使用 2M PHY、+8 dBm 发射功率；分体中央侧在空闲 2 分钟后进入休眠。
- 左右半电池 ADC 统一由 `keyboard.toml` 配置，保留原有引脚和分压校准，默认每 5 分钟采样。
- 双侧电量通知等待新的按键，减少后台电量变化直接唤醒主机的情况；右半通知使用按键触发时
  的最新缓存。主机主动读取任意一侧电量时，直接刷新对应属性为最新缓存值，无需等待按键。
- 灯效与渐变每 33 ms 刷新，常亮或熄灭时降为每秒检查；状态事件可提前触发刷新。
  休眠时双灯渐灭，完全熄灭后关闭灯珠电源并暂停轮询，等待事件唤醒。
- 低电量恢复后重置提醒计划，再次降至阈值可立即提醒；呼吸谷底和闪烁暗相仍保持动画刷新。

### 状态灯测试与 Flash 保护

- 灯效状态抽离为不依赖硬件的模块，宿主测试直接编译实际固件状态逻辑。
- Flash 固件区域为 `[0x1000, 0xA0000)`，共 636 KiB；配置存储沿用
  `[0xA0000, 0xC0000)`，共 128 KiB（32 个 4 KiB 扇区）。链接器限制固件大小，防止覆盖配置区。
- Flash 按存储格式版本识别配置。初始化完成后才提交版本标记，中途断电会在下次启动时重新初始化。
- 启动时检查或初始化 Flash 失败会重试一次。若存储仍不可用，本次开机暂停 Vial 配置与 Flash 保存，重启后重新检查。
- CI 包含格式、Clippy、灯效状态、Flash 分区与链接边界、RMK 库测试，以及固件和 UF2 构建。

更改分区时必须同步修改 `keyboard.toml` 和 `memory.x`，并单独评估已有配置的迁移。
`vendor/rmk` 和 `vendor/rmk-macro` 基于上游 `rmk-v0.9.0` 标签，保留了上述本地适配；
升级依赖时需一并核对。

## 状态灯说明

每半边有两颗独立控制的 WS2812 灯。同一颗灯按表格从上到下匹配优先级，临时提示结束后
自动恢复下一项有效状态；休眠时统一渐灭。

### 左半（中央侧）

| 灯位 | 状态 | 表现 |
| --- | --- | --- |
| 内侧（右灯） | USB 充电 | 绿色呼吸；电量达到 95% 后常绿 3 秒，再熄灭 |
| 内侧（右灯） | 低电量 | 电量不高于 20% 时，珊瑚色双闪持续 5 秒，随后间隔 5 分钟再次提醒 |
| 内侧（右灯） | 分体连接 | 断开时蓝色呼吸，最长 60 秒；连接后常蓝 3 秒 |
| 外侧（左灯） | 蓝牙连接／槽位切换 | 连接或切换槽位后，以当前槽位颜色提示 3 秒 |
| 外侧（左灯） | 蓝牙广播 | 以当前槽位颜色呼吸，最长 60 秒 |
| 外侧（左灯） | Caps Lock | 开启时保持琥珀色 |

蓝牙槽位 1–5 依次使用紫水晶、祖母绿、蓝宝石、石榴石和钻石色，亮度经过限制。

### 右半（外设侧）

| 灯位 | 状态 | 表现 |
| --- | --- | --- |
| 内侧 | USB 充电 | 绿色呼吸；电量达到 95% 后常绿 3 秒，再熄灭 |
| 内侧 | 低电量 | 电量不高于 20% 时，珊瑚色双闪持续 5 秒，随后间隔 5 分钟再次提醒 |
| 外侧 | 分体连接 | 断开时蓝色呼吸，最长 60 秒；连接后常蓝 3 秒 |

呼吸使用 Gamma 亮度校正，状态切换平滑渐变。

## 构建与刷机

### 升级前备份

左右半必须使用同一套固件。首次从旧版升级到存储格式 1 时，固件会清空配置并加载默认键位。
刷机后请删除电脑或手机上的旧蓝牙配对，再重新配对。
后续使用同一存储格式的固件时，会保留 Vial 设置和蓝牙配对。
刷机前可导出 `.vil` 配置并保存旧版左右半 UF2，以便恢复或回退。

### 本地构建

需要 Rust `stable`（组件和目标见 `rust-toolchain.toml`）、`flip-link`、`cargo-make`
以及 UF2 转换工具。`Makefile.toml` 定义了构建任务和相关工具的安装依赖。

还需要完整的 ARM GNU 裸机工具链（`arm-none-eabi-gcc`、`arm-none-eabi-ar` 及 newlib 头文件）。
新版蓝牙加密依赖会编译 C／汇编，请将工具链的 `bin` 目录加入 `PATH`。

[`keyboard.toml`](keyboard.toml) 的默认按键和旋钮来自 Vial 导出。数字层、功能层和蓝牙层的旋钮
继承基础层的音量与切歌操作。`MO(4)` 进入默认全透明的自定义层，可在 Vial 中添加快捷键。
键盘会优先使用已保存的键位，存储初始化后才加载固件默认键位。
按需修改配置后，在仓库根目录生成刷机文件。

```sh
cargo make uf2
```

### 下载与刷机

从 [GitHub Releases](https://github.com/WangNingkai/rmk-cornix/releases) 下载版本 ZIP 和 `SHA256SUMS`。
在下载目录运行 `sha256sum -c SHA256SUMS`（macOS 可用 `shasum -a 256 -c SHA256SUMS`），
校验后解压。测试版也可在 GitHub Actions 手动运行 **Build RMK firmware**，从同一次运行
下载 `rmk-cornix-firmware` 产物。将左右半分别置于 Bootloader 模式，刷入各自的 UF2。

| 文件 | 用途 |
| --- | --- |
| `rmk-cornix-central.uf2` | 左半（中央侧）刷机 |
| `rmk-cornix-peripheral.uf2` | 右半（外设侧）刷机 |
| `rmk-cornix-central.hex`、`rmk-cornix-peripheral.hex` | 对应半边的 HEX 固件 |

`cargo build --release --locked` 生成 `target/` 下的 ELF，`cargo make uf2` 完成 UF2 转换。

推送 `v主版本.次版本.修订号` 格式的标签会触发正式发布。检查和构建通过后，CI 将同一次运行
的左右半 UF2 打成一个 ZIP，连同校验文件一并发布。

参考：[RMK 官方迁移指南](https://rmk.rs/docs/migration/v08_v09)。

## 检查与测试

以下命令均在仓库根目录执行。

### 固件静态检查与构建

```sh
cargo fmt --check
cargo clippy --release --bins --locked -- -D warnings
cargo build --release --locked
```

### 灯效状态测试

直接编译固件使用的状态模块，无需 nRF 硬件或额外依赖：

```sh
mkdir -p target
rustfmt --edition 2021 --check tests/ws2812.rs
rustc --edition 2021 --test tests/ws2812.rs -o target/ws2812-tests
./target/ws2812-tests
```

覆盖低电量提醒重新触发、双侧休眠渐灭与暂停刷新、提示到期恢复 Caps Lock、充电优先级
及动画暗相刷新。PWM 波形和实际功耗仍需实机验证。

### Flash 分区与链接边界

需要 Python 3.11+ 和 ARM GNU 工具链中的 `arm-none-eabi-as`、`arm-none-eabi-ld`：

```sh
python3 tests/test_memory_layout.py
```

检查 `keyboard.toml` 与 `memory.x` 的分区一致性，并使用实际链接脚本验证固件恰好占满
区域时能够链接、超出一个字节时链接失败。缺少 ARM 工具链时，链接边界测试会报错。

### RMK 库测试

使用 nextest 隔离全局模拟时钟，并使用独立夹具 `tests/rmk.toml`。安装
`cargo-nextest 0.9.152` 后执行，`--target` 替换为本机宿主平台（下例为 Apple Silicon macOS，
Linux CI 使用 `x86_64-unknown-linux-gnu`）：

```sh
KEYBOARD_TOML_PATH="$PWD/tests/rmk.toml" cargo nextest run \
  --manifest-path vendor/rmk/Cargo.toml --target aarch64-apple-darwin \
  --no-default-features --features std,log,split,_ble,vial,storage --lib
```

### 实机验收

- 双侧按键及旋钮、Vial 配置保存与恢复。
- 使用同一存储格式的两次独立构建互刷，确认改过的 Vial 键位和蓝牙配对保留。
- 左右半电量读取、通知时机和状态灯优先级。
- 蓝牙槽位切换、单侧重启和断线重连。
- 保持左半与主机的蓝牙连接，右半断电后等左半休眠，再仅给右半上电，检查右半是否自行
  恢复输入，并记录重连时间和右半离线期间的左半功耗。
- 右半保持开机，左半关机约 6 分钟后重新开启。期间不按右半，记录它是否自行重连及所需时间。
- 电脑休眠期间是否被意外唤醒、键盘休眠后的首次输入。
- 灯效 PWM 波形及实际功耗。
