# RMK Firmware for Cornix

[简体中文](README.md)

This repository provides unofficial [RMK](https://rmk.rs/) firmware for the Jezail Funder Cornix keyboard.
It uses RMK `0.9.0`, the nRF52840, and the Adafruit Bootloader.

## Features and compatibility

- Supports all keys, both rotary encoders, and keymap editing in Vial.
- Reports battery levels for both halves. The onboard WS2812 LEDs indicate battery, connection, and Caps Lock status.
- The Vial layout is broadly compatible with the stock firmware. You can import an existing `.vil` file. Check macros,
  combos, Tap Dance, and encoder bindings individually and reconfigure them as needed.
- The onboard RGB LEDs are reserved for status indicators.

## Connection, power, and storage

### Connection stability and input recovery

- Split service discovery and writes have timeouts. The right half waits up to five seconds for a write. A failed or timed-out
  write ends the session, disconnects the link, and lets the halves reconnect.
- After a disconnect, the firmware waits for space in the event queue, releases held split keys, and then reconnects.
- While the left half is asleep and a host connection is available, it still attempts to reconnect to the right half. Each
  connection attempt or advertising wait lasts up to one second, followed by a five-second pause. Without a host connection,
  the left half waits for wake-up before retrying.
- Switching Bluetooth profiles clears held keys, temporary combo layers, modifiers, and mouse state. The firmware ignores
  later releases from those keys and keeps explicitly selected persistent layers.
- Reboot and Bootloader actions are forwarded to the half where the triggering key was pressed.

### Battery and power saving

- BLE uses 2M PHY and +8 dBm transmit power. The split central sleeps after two minutes of inactivity.
- [`keyboard.toml`](keyboard.toml) configures both halves' battery ADC pins and voltage divider calibration. The default
  sampling interval is five minutes.
- Battery notifications wait for a new keypress to reduce unintended host wake-ups. Right-half notifications use the latest
  cached reading at that point. When the host reads either battery level directly, the firmware refreshes that attribute from
  the corresponding cache immediately.
- Animations and fades refresh every 33 ms. The firmware checks steady or off LEDs once per second, and status events can
  trigger an earlier refresh. During sleep both LEDs fade out, then the firmware cuts LED power and waits for events.
- When the battery recovers, the firmware resets the low-battery reminder so it can alert immediately if the level falls again.
  Breathing and blinking continue to refresh during their dark phases.

### Status LED tests and Flash protection

- The LED state machine runs independently of the hardware so host-side tests can exercise the same logic used by the firmware.
- The firmware occupies `[0x1000, 0xA0000)`, a 636 KiB region. Configuration storage remains in `[0xA0000, 0xC0000)`,
  which is 128 KiB across 32 sectors of 4 KiB each. The linker enforces the firmware limit to protect saved configuration.
- Flash configuration has a format version. The firmware records it when initialization completes and retries on the next
  boot if power is interrupted during initialization.
- A failed Flash check or initialization is retried once at startup. If storage is still unavailable, Vial configuration and
  Flash saving pause until the next boot.
- CI checks formatting, Clippy, LED state, Flash partition boundaries, and the RMK library before building the firmware and UF2 files.

If you change the Flash layout, update both [`keyboard.toml`](keyboard.toml) and [`memory.x`](memory.x) and review how to
migrate existing configuration. `vendor/rmk` and `vendor/rmk-macro` are based on the upstream `rmk-v0.9.0` tag. Review the
local patches when upgrading RMK.

## Status LEDs

Each half has two independently controlled WS2812 LEDs. The rows for each LED are in priority order. When a temporary
indication expires, the LED resumes the next applicable state. Both LEDs fade out during sleep.

### Left half (central)

| LED | State | Indication |
| --- | --- | --- |
| Inner (right) | USB charging | Breathes green. At 95%, it stays green for three seconds, then turns off. |
| Inner (right) | Low battery | At 20% or below, gives two coral flashes at a time for five seconds. The reminder repeats after a five-minute interval. |
| Inner (right) | Split connection | Breathes blue for up to 60 seconds when disconnected. It stays blue for three seconds after connecting. |
| Outer (left) | Bluetooth connection or profile switch | Shows the current profile color for three seconds. |
| Outer (left) | Bluetooth advertising | Breathes in the current profile color for up to 60 seconds. |
| Outer (left) | Caps Lock | Stays amber while Caps Lock is on. |

Bluetooth profiles 1–5 use amethyst, emerald, sapphire, garnet, and diamond colors, in that order. Brightness is limited.

### Right half (peripheral)

| LED | State | Indication |
| --- | --- | --- |
| Inner | USB charging | Breathes green. At 95%, it stays green for three seconds, then turns off. |
| Inner | Low battery | At 20% or below, gives two coral flashes at a time for five seconds. The reminder repeats after a five-minute interval. |
| Outer | Split connection | Breathes blue for up to 60 seconds when disconnected. It stays blue for three seconds after connecting. |

Breathing uses gamma correction, and LED states transition with a fade.

## Building and flashing

### Back up before upgrading

Flash both halves with firmware from the same build. Upgrading from an older release to storage format 1 clears the saved
configuration and loads the default keymap. Remove the old Bluetooth pairing from your computer or phone and pair again.
Future builds using the same storage format retain Vial settings and Bluetooth pairings. You can export your Vial `.vil`
configuration and keep the previous pair of UF2 files before flashing for restoration or rollback.

### Build locally

You need Rust `stable`, `flip-link`, `cargo-make`, and the UF2 conversion tools. The Rust components and target are listed in
[`rust-toolchain.toml`](rust-toolchain.toml). The build tasks and tool installation steps are defined in
[`Makefile.toml`](Makefile.toml).

You also need the ARM GNU bare-metal toolchain, including `arm-none-eabi-gcc`, `arm-none-eabi-ar`, and the newlib headers.
Add the toolchain's `bin` directory to `PATH` so the Bluetooth cryptography dependency can compile its C and assembly code.

The default keys and encoders in [`keyboard.toml`](keyboard.toml) come from a Vial export. Encoders on the number, function,
and Bluetooth layers inherit the base layer's volume and track controls. `MO(4)` opens the custom layer, which is transparent
by default and can be configured in Vial. Saved bindings take priority until the firmware initializes storage.
Edit the configuration as needed, then build the flashable files from the repository root.

```sh
cargo make uf2
```

### Download and flash

Download the versioned ZIP and `SHA256SUMS` from [GitHub Releases](https://github.com/WangNingkai/rmk-cornix/releases).
Run `sha256sum -c SHA256SUMS` in the download directory (`shasum -a 256 -c SHA256SUMS` on macOS), then extract the ZIP.
For a test build, run **Build RMK firmware** manually in GitHub Actions and download the `rmk-cornix-firmware` artifact
from that run. Put each half in Bootloader mode and flash its corresponding UF2.

| File | Use |
| --- | --- |
| `rmk-cornix-central.uf2` | Flash the left half (central) |
| `rmk-cornix-peripheral.uf2` | Flash the right half (peripheral) |
| `rmk-cornix-central.hex`, `rmk-cornix-peripheral.hex` | Intel HEX images for the corresponding halves |

`cargo build --release --locked` creates the ELF files under `target/`. `cargo make uf2` also converts them to UF2.

Pushing a `vMAJOR.MINOR.PATCH` tag triggers a release. After the checks and build pass, CI packages both UF2 files
from that run into one ZIP and publishes it with the checksum file.

For details on the RMK upgrade, see the [official migration guide](https://rmk.rs/docs/migration/v08_v09).

## Checks and tests

Run these commands from the repository root.

### Firmware checks and build

```sh
cargo fmt --check
cargo clippy --release --bins --locked -- -D warnings
cargo build --release --locked
```

### Status LED tests

Compile the firmware's LED state module on the host. You only need the Rust toolchain for these tests.

```sh
mkdir -p target
rustfmt --edition 2021 --check tests/ws2812.rs
rustc --edition 2021 --test tests/ws2812.rs -o target/ws2812-tests
./target/ws2812-tests
```

The tests cover recurring low-battery alerts, sleep fade-out and paused polling, restoring Caps Lock after a temporary
indication, charging priority, and refresh during dark animation phases. Check PWM timing and power draw on the hardware.

### Flash partitions and linker limits

This test requires Python 3.11+ and `arm-none-eabi-as` and `arm-none-eabi-ld` from the ARM GNU toolchain.

```sh
python3 tests/test_memory_layout.py
```

It checks that `keyboard.toml` and `memory.x` agree and uses the actual linker script to verify that a firmware image at the
exact size limit links successfully and a one-byte overflow is rejected.

### RMK library tests

Install `cargo-nextest 0.9.152` to isolate tests that use the same simulated clock. The tests use `tests/rmk.toml`.
Use your host's target triple. The example is for Apple Silicon macOS. Linux CI uses `x86_64-unknown-linux-gnu`.

```sh
KEYBOARD_TOML_PATH="$PWD/tests/rmk.toml" cargo nextest run \
  --manifest-path vendor/rmk/Cargo.toml --target aarch64-apple-darwin \
  --no-default-features --features std,log,split,_ble,vial,storage --lib
```

### Hardware checks

- Check both halves' keys and encoders, and save and restore the Vial configuration.
- Flash two independent builds using the same storage format and verify that custom Vial bindings and Bluetooth pairings persist.
- Check battery readings, notification timing, and status LED priority on both halves.
- Switch Bluetooth profiles, reboot either half, and check reconnection after a disconnect.
- Keep the left half connected to a Bluetooth host, turn off the right half, and wait for the left half to sleep. Turn the right
  half back on using only its power switch. Check that input resumes, and record the time to reconnect and the left half's power
  draw while the right half is offline.
- Leave the right half powered, turn off the left half for about six minutes, then turn it back on. Leave the right half's keys
  untouched and record whether it reconnects on its own and how long it takes.
- Check for unintended host wake-ups and verify the first keystroke after keyboard sleep.
- Measure the LED PWM waveform and actual power consumption.
