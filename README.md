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
- BLE is configured for balanced daily use: 2M PHY, +4 dBm TX power, and split
  central sleep after 5 minutes of inactivity.
- Status LEDs use an adaptive refresh: they run smooth, gamma-corrected
  breathing with soft fade in/out while active, and drop to a slow idle tick
  once everything settles to off, so idle wakeups stay low.
- Status LEDs are power-limited. Connection/profile events show for 3 seconds,
  advertising/disconnected breathing stops after 60 seconds, and low battery
  reminders pulse briefly every 5 minutes.

# Usage

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
