# Cornix 键盘 RMK 固件

[English](README.en.md)

适用于 Jezail Funder Cornix 的非官方 [RMK](https://rmk.rs/) 固件，基于 RMK `0.9.0`，使用 nRF52840 和 Adafruit UF2 Bootloader。

- 左半通过 USB 或蓝牙连接主机，右半通过蓝牙连接左半。
- 支持全部按键、双侧旋钮、五个蓝牙槽位和 Vial 改键。
- 可读取双半电量。板载 WS2812 用于显示电量、连接状态和 Caps Lock。

## 下载与刷机

1. 在 Vial 中导出 `.vil` 备份，并保留旧版左右半的 UF2 文件。
2. 从 [GitHub Releases](https://github.com/WangNingkai/rmk-cornix/releases) 下载同一版本的 ZIP 和 `SHA256SUMS`。
3. 在下载目录校验 ZIP，再解压。

   ```sh
   # Linux
   sha256sum -c SHA256SUMS

   # macOS
   shasum -a 256 -c SHA256SUMS
   ```

   Windows 可在 PowerShell 中运行 `Get-FileHash .\rmk-cornix-*.zip -Algorithm SHA256`，将对应 ZIP 的结果与 `SHA256SUMS` 比对。

4. 将两半分别置于 UF2 Bootloader 模式，把对应文件复制到该半边的 Bootloader 磁盘。左右半须使用同一批构建产物。

   | 文件 | 对应半边 |
   | --- | --- |
   | `rmk-cornix-central.uf2` | 左半（中央侧） |
   | `rmk-cornix-peripheral.uf2` | 右半（外设侧） |

首次从旧版升级到**存储格式 1**时，固件会清空旧配置并加载默认键位。刷机后请在电脑或手机上删除旧蓝牙配对，再重新配对。默认设置下，后续升级到同一存储格式的固件会保留 Vial 设置和蓝牙配对。

需要测试构建时，可在 GitHub Actions 手动运行 **Build RMK firmware**，从同一次运行下载 `rmk-cornix-firmware` 产物。

## 使用

### Vial 和默认键位

使用 USB 将左半接到电脑，再打开 Vial。

默认键位与旋钮映射见 [`keyboard.toml`](keyboard.toml)。基础层的左右旋钮分别控制音量和切歌，数字层、功能层和蓝牙层会继承这些操作。`MO(4)` 对应自定义层，该层默认继承下层动作。固件优先使用已经保存的键位，初始化存储时加载默认键位。

可以导入原厂固件导出的 `.vil` 文件。导入后请核对按键、旋钮、宏、组合键和 Tap Dance，按需重新配置。Vial 修改键位或旋钮时，固件在 Flash 写入成功后才应用修改和回复。

### 蓝牙槽位与分体重连

- 短按已配置的 `BT0` 至 `BT4` 键可切换槽位。按住槽位键 5 秒会清除该槽位的旧配对并切换过去，以便重新配对。松开层键或操作普通按键时，槽位键仍会继续计时。
- 右半快速广播超时后会进入低频重连，每轮等待 20 秒，再以 200 ms 间隔广播最多 8 秒。低频广播期间只接受已保存的左半地址发起的连接。按下右半按键可立即恢复快速广播。右半尚未保存左半地址时，由本侧按键唤醒广播。
- 左半休眠时，如果仍连接主机，会继续尝试重连右半。每次尝试最长 1 秒，结束后等待 5 秒。左半休眠且无主机连接时，可按左半任意键唤醒。

### 状态灯

每半边有两颗状态灯。对于同一颗灯，表中靠上的状态优先。临时提示结束后，灯会恢复下一个有效状态。进入休眠时两颗灯渐灭。

| 灯位 | 状态 | 提示 |
| --- | --- | --- |
| 双半内侧 | USB 充电 | 绿色呼吸。电量达到 95% 后常绿 3 秒，再熄灭。 |
| 双半内侧 | 低电量 | 电量不高于 20% 时，珊瑚色双闪持续 5 秒，间隔 5 分钟后再次提醒。 |
| 左半内侧、右半外侧 | 分体连接 | 断开时蓝色呼吸，最长 60 秒。连接后常蓝 3 秒。 |
| 右半外侧 | 按键唤醒重连 | 右半低频等待期间按下任意键，蓝灯短闪一次，确认已恢复快速广播。 |
| 左半外侧 | 蓝牙连接或槽位切换 | 显示当前槽位的颜色 3 秒。 |
| 左半外侧 | 蓝牙广播 | 以当前槽位颜色呼吸，最长 60 秒。 |
| 左半外侧 | Caps Lock | 开启时常亮琥珀色。 |

左半内侧为右灯，外侧为左灯。`BT0` 至 `BT4` 依次对应紫水晶、祖母绿、蓝宝石、石榴石和钻石色。呼吸灯采用 Gamma 亮度校正。

## 从源码构建

- Rust `stable`。目标和工具链组件见 [`rust-toolchain.toml`](rust-toolchain.toml)。
- `cargo-make`。[`Makefile.toml`](Makefile.toml) 定义了构建任务及 `flip-link`、UF2 转换工具的安装依赖。
- ARM GNU 裸机工具链。安装 `arm-none-eabi-gcc`、`arm-none-eabi-ar` 和 newlib 头文件，并将工具链的 `bin` 加入 `PATH`。蓝牙加密依赖构建时会用到这些工具。

在仓库根目录执行以下命令。

```sh
cargo make uf2
```

HEX 和 UF2 文件生成在仓库根目录。单独运行 `cargo build --release --locked` 时，左右半 ELF 生成在 `target/thumbv7em-none-eabihf/release/`。

主要配置文件是 [`keyboard.toml`](keyboard.toml)（引脚、键位、电池 ADC 和蓝牙）、[`vial.json`](vial.json)（Vial 布局）和 [`memory.x`](memory.x)（链接内存布局）。

## 测试

从仓库根目录运行。[`build.yml`](.github/workflows/build.yml) 定义了 CI 检查及固件构建流程。

### 固件检查

```sh
cargo fmt --check
cargo clippy --release --bins --locked -- -D warnings
cargo build --release --locked
```

### 状态灯和 Flash 分区

```sh
mkdir -p target
rustfmt --edition 2021 --check tests/ws2812.rs
rustc --edition 2021 --test tests/ws2812.rs -o target/ws2812-tests
./target/ws2812-tests

python3 tests/test_memory_layout.py
```

状态灯测试在宿主机编译固件的实际状态模块，覆盖低电量提醒、休眠渐灭、提示优先级和动画刷新。Flash 测试需要 Python 3.11+、`arm-none-eabi-as` 和 `arm-none-eabi-ld`，检查分区配置一致性，并验证固件恰好占满分区时能链接、超出一个字节时链接失败。

### RMK 单元与集成测试

使用 `cargo-nextest 0.9.152` 隔离使用全局模拟时钟的测试。下面以 Apple Silicon macOS 为例。Linux CI 使用 `x86_64-unknown-linux-gnu`。

```sh
KEYBOARD_TOML_PATH="$PWD/tests/rmk.toml" cargo nextest run \
  --manifest-path vendor/rmk/Cargo.toml --target aarch64-apple-darwin \
  --no-default-features --features std,log,split,_ble,vial,storage --lib --test integration
```

### 实机验收

- [ ] 在目标设备上验证蓝牙配对与唤醒。通过 USB 验证有线输入和 Vial 保存与恢复，并检查双半按键和旋钮。
- [ ] 用相同存储格式的两份独立构建互刷，确认 Vial 键位、旋钮设置和蓝牙配对保留。
- [ ] 验证蓝牙槽位短按切换。长按槽位键时先松开层键，检查满 5 秒后能否重新配对。
- [ ] 左半保持与主机连接并进入休眠，再给右半上电。检查能否自行恢复输入，记录重连时间和左半功耗。
- [ ] 右半保持开机，左半关机约 6 分钟后重新开启。期间不按右半，记录自动重连时间和右半待机电流，确认只有正常连接提示。另测右半等待期间按键是否短闪一次蓝灯并恢复快速广播。
- [ ] 检查双半电量读取、通知时机、指示灯优先级与 PWM 波形，以及电脑休眠后的意外唤醒和键盘首次输入。

## 固件实现

- 分体服务发现和写入均设有超时。右半写入限时 5 秒，出错会断开并重连。断线后等待事件队列空位并释放分体按键，再开始重连。
- 切换蓝牙槽位会清理按键、临时层、修饰键和鼠标状态，保留常驻层并忽略切换前已释放按键的后续松开。重启和 Bootloader 动作发送到触发按键所在的半边。
- 蓝牙连接按配置请求 2M PHY，分体广播使用 1M PHY。发射功率设为 +8 dBm，左半空闲 2 分钟后休眠。
- 双半 ADC 默认每 5 分钟采样。电量通知等待新的按键，主机主动读取时直接取缓存。灯效和渐变每 33 ms 刷新，静止时每秒检查，休眠渐灭后关闭灯珠电源并暂停轮询。
- Flash 初始化写完默认配置后才记录存储版本。中途断电后，下一次启动会重新初始化。检查或初始化出错会重试一次，持续失败时暂停本次启动的 Vial 配置和 Flash 写入。VIA 批量键位读写会验证长度、偏移和键位范围。

| Flash 区域 | 地址范围 | 容量 |
| --- | --- | --- |
| 固件 | `[0x1000, 0xA0000)` | 636 KiB |
| 配置存储 | `[0xA0000, 0xC0000)` | 128 KiB，32 个 4 KiB 扇区 |

链接器限制固件尺寸。调整分区时须同步修改 [`keyboard.toml`](keyboard.toml) 和 [`memory.x`](memory.x)，并核对已有配置的迁移方式。

## 发布

推送 `v主版本.次版本.修订号` 格式的标签后，CI 会在检查和构建通过时把同一次构建的左右半 UF2 打包为一个 ZIP，并附上 `SHA256SUMS`。

[`vendor/rmk`](vendor/rmk) 和 [`vendor/rmk-macro`](vendor/rmk-macro) 基于上游 `rmk-v0.9.0`，包含 Cornix 的本地适配。升级 RMK 时可对照本地改动和 [RMK 官方迁移指南](https://rmk.rs/docs/migration/v08_v09)。

## 许可证

项目在 [`Cargo.toml`](Cargo.toml) 中声明使用 `MIT OR Apache-2.0`。RMK 的许可证文本见 [MIT](vendor/rmk/LICENSE-MIT) 和 [Apache-2.0](vendor/rmk/LICENSE-APACHE)。
