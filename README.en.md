# RMK Firmware for Cornix

[简体中文](README.md)

Unofficial [RMK](https://rmk.rs/) firmware for the Jezail Funder Cornix, based on RMK `0.9.0` and built for the nRF52840 with the Adafruit UF2 Bootloader.

- The left half connects to the host over USB or Bluetooth. The right half connects to the left over Bluetooth.
- Supports all keys, both rotary encoders, five Bluetooth profiles, and remapping in Vial.
- Reports battery levels for both halves. The onboard WS2812 LEDs show battery, connection, and Caps Lock status.

## Download and flash

1. Export a Vial `.vil` backup and keep the previous UF2 files for both halves.
2. Download the ZIP and `SHA256SUMS` for the same version from [GitHub Releases](https://github.com/WangNingkai/rmk-cornix/releases).
3. Verify the ZIP in the download directory, then extract it.

   ```sh
   # Linux
   sha256sum -c SHA256SUMS

   # macOS
   shasum -a 256 -c SHA256SUMS
   ```

   On Windows, run `Get-FileHash .\rmk-cornix-*.zip -Algorithm SHA256` in PowerShell and compare the ZIP's hash with `SHA256SUMS`.

4. Put each half into UF2 Bootloader mode and copy its UF2 file to the Bootloader drive. Use files from the same build on both halves.

   | File | Keyboard half |
   | --- | --- |
   | `rmk-cornix-central.uf2` | Left (central) |
   | `rmk-cornix-peripheral.uf2` | Right (peripheral) |

The first upgrade from an older release to **storage format 1** clears the saved configuration and loads the default keymap. Remove the old Bluetooth pairing from your computer or phone, then pair again. With the default storage settings, future builds using the same format retain Vial settings and Bluetooth pairings.

For a test build, run **Build RMK firmware** manually in GitHub Actions and download the `rmk-cornix-firmware` artifact from that run.

## Usage

### Vial and the default keymap

Pair and connect the left half to your computer over Bluetooth, then open Vial to remap keys. You can also use Vial with the left half connected over USB.

See [`keyboard.toml`](keyboard.toml) for the default keymap and encoder bindings. On the base layer, the left encoder controls volume and the right encoder changes tracks. The number, function, and Bluetooth layers inherit these actions. `MO(4)` opens the custom layer, which inherits lower-layer actions by default. The firmware uses saved bindings when available and loads the defaults when it initializes storage.

You can import a `.vil` file exported from the stock firmware. Check keys, encoders, macros, combos, and Tap Dance after import and configure them as needed. When you change a key or encoder in Vial, the firmware applies the change and replies after a successful Flash write.

### Bluetooth profiles and split reconnection

- Tap a configured `BT0` through `BT4` key to switch profiles. Hold it for five seconds to clear that profile's old pairing and switch to it for pairing again. The timer keeps running when you release the layer key or use an ordinary key.
- After fast advertising times out, the right half starts a repeating cycle. It waits 20 seconds, then advertises for up to eight seconds at 200 ms intervals. During these windows it accepts connections from the saved left-half address. Pressing a key on the right half resumes fast advertising immediately. If the right half has no saved left-half address, press one of its keys to start advertising again.
- While the left half is asleep with a host connection, it keeps trying to reconnect to the right half. Each attempt lasts up to one second, followed by a five-second pause. When the left half is asleep without a host connection, press a key on the left half to wake it.

### Status LEDs

Each half has two status LEDs. For any one LED, the higher row in the table takes priority. When a temporary indication expires, the LED returns to the next applicable state. Both LEDs fade out during sleep.

| LED | State | Indication |
| --- | --- | --- |
| Both inner LEDs | USB charging | Breathes green. At 95%, it stays green for three seconds, then turns off. |
| Both inner LEDs | Low battery | At 20% or below, flashes coral twice at a time for five seconds. It repeats after five minutes. |
| Left inner and right outer | Split connection | Breathes blue for up to 60 seconds when disconnected. Stays blue for three seconds after connecting. |
| Right outer | Key press during idle reconnect | Flashes blue once when a key press resumes fast advertising. |
| Left outer | Bluetooth connection or profile switch | Shows the current profile color for three seconds. |
| Left outer | Bluetooth advertising | Breathes in the current profile color for up to 60 seconds. |
| Left outer | Caps Lock | Stays amber while Caps Lock is on. |

On the left half, the inner LED is on the right and the outer LED is on the left. `BT0` through `BT4` use amethyst, emerald, sapphire, garnet, and diamond respectively. Breathing uses gamma correction.

## Build from source

- Rust `stable`. See [`rust-toolchain.toml`](rust-toolchain.toml) for the target and components.
- `cargo-make`. [`Makefile.toml`](Makefile.toml) defines the build tasks and installs tools such as `flip-link` and the UF2 converter as needed.
- The ARM GNU bare-metal toolchain, including `arm-none-eabi-gcc`, `arm-none-eabi-ar`, and the newlib headers. Add its `bin` directory to `PATH` so the Bluetooth cryptography dependency can build.

Run from the repository root.

```sh
cargo make uf2
```

The HEX and UF2 files appear in the repository root. Running `cargo build --release --locked` on its own produces both ELF files in `target/thumbv7em-none-eabihf/release/`.

The main configuration files are [`keyboard.toml`](keyboard.toml) for pins, bindings, battery ADC, and Bluetooth, [`vial.json`](vial.json) for the Vial layout, and [`memory.x`](memory.x) for the linker memory layout.

## Tests

Run these commands from the repository root. [`build.yml`](.github/workflows/build.yml) defines the CI checks and firmware build.

### Firmware checks

```sh
cargo fmt --check
cargo clippy --release --bins --locked -- -D warnings
cargo build --release --locked
```

### Status LEDs and Flash partitions

```sh
mkdir -p target
rustfmt --edition 2021 --check tests/ws2812.rs
rustc --edition 2021 --test tests/ws2812.rs -o target/ws2812-tests
./target/ws2812-tests

python3 tests/test_memory_layout.py
```

The LED tests compile the firmware's actual state logic on the host. They cover low-battery alerts, sleep fade-out, indication priority, and animation refresh. The Flash tests require Python 3.11+, `arm-none-eabi-as`, and `arm-none-eabi-ld`. They check the partition configuration and confirm that an image at the size limit links successfully and a one-byte overflow fails.

### RMK unit and integration tests

Use `cargo-nextest 0.9.152` to isolate tests that share a simulated clock. This example uses the Apple Silicon macOS target. Linux CI uses `x86_64-unknown-linux-gnu`.

```sh
KEYBOARD_TOML_PATH="$PWD/tests/rmk.toml" cargo nextest run \
  --manifest-path vendor/rmk/Cargo.toml --target aarch64-apple-darwin \
  --no-default-features --features std,log,split,_ble,vial,storage --lib --test integration
```

### Hardware checks

- [ ] Check Bluetooth pairing and wake-up, USB input, both halves' keys, and both encoders on the target devices. Check Vial save and restore over both Bluetooth and USB.
- [ ] Flash two independent builds using the same storage format and verify that Vial key and encoder bindings and Bluetooth pairings persist.
- [ ] Tap a Bluetooth profile key to switch profiles. Hold the profile key and release the layer key during the hold. Verify that pairing is available after five seconds.
- [ ] Keep the left half connected to a host and let it sleep, then turn on the right half. Check that input resumes automatically, and record reconnection time and left-half power draw.
- [ ] Leave the right half on and turn the left half off for about six minutes. Turn the left half back on and leave the right half's keys untouched. Record automatic reconnection time and right-half standby current, and check for the normal connection indication. Then check that pressing a key during the wait flashes blue once and resumes fast advertising.
- [ ] Check both battery readings, notification timing, LED priority, and PWM waveforms. Check for unintended host wake-ups and verify the first key after keyboard sleep.

## Firmware details

- Split service discovery and writes have timeouts. The right half has up to five seconds to write, and it disconnects and reconnects on failure. After losing the link, the firmware waits for event queue space and releases split keys before reconnecting.
- Switching Bluetooth profiles clears held keys, temporary layers, modifiers, and mouse state. The firmware keeps persistent layers and ignores later releases from the keys it just cleared. Reboot and Bootloader actions are sent to the half where the triggering key was pressed.
- Bluetooth connections request 2M PHY as configured. Split advertising uses 1M PHY, transmit power is set to +8 dBm, and the left half sleeps after two minutes of inactivity.
- The battery ADCs sample every five minutes by default. Notifications wait for a new key press, and direct reads use the latest cached reading. Animations and fades refresh every 33 ms. Steady LEDs are checked once per second. After fading out in sleep, the LEDs power off and polling pauses.
- Flash initialization writes the format marker after saving the defaults. An interruption during initialization is retried on the next boot. A Flash check or initialization error is retried once. If storage still fails, Vial configuration and Flash writes pause for that boot. VIA bulk keymap reads and writes validate length, offset, and range.

| Flash region | Address range | Size |
| --- | --- | --- |
| Firmware | `[0x1000, 0xA0000)` | 636 KiB |
| Configuration storage | `[0xA0000, 0xC0000)` | 128 KiB, 32 sectors of 4 KiB |

The linker enforces the firmware size limit. If you change the partitions, update [`keyboard.toml`](keyboard.toml) and [`memory.x`](memory.x) together and check how saved configurations will migrate.

## Releases

Pushing a `vMAJOR.MINOR.PATCH` tag starts a release build. Once checks and builds pass, CI packages both UF2 files from that build in a single ZIP and publishes it with `SHA256SUMS`.

[`vendor/rmk`](vendor/rmk) and [`vendor/rmk-macro`](vendor/rmk-macro) are based on the upstream `rmk-v0.9.0` tag and include Cornix-specific changes. Review the local patches and the [official RMK migration guide](https://rmk.rs/docs/migration/v08_v09) when upgrading RMK.

## License

The project declares `MIT OR Apache-2.0` in [`Cargo.toml`](Cargo.toml). The vendored RMK license texts are available under [MIT](vendor/rmk/LICENSE-MIT) and [Apache-2.0](vendor/rmk/LICENSE-APACHE).
