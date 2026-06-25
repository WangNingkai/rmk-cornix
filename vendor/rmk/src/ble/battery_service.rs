use core::sync::atomic::Ordering;

use embassy_sync::signal::Signal;
use embassy_time::Timer;
use trouble_host::prelude::*;

use super::ble_server::Server;
use crate::input_device::battery::{BATTERY_UPDATE, BatteryState, BatteryUpdate};
use crate::keyboard::{KEY_PRESS_SEQUENCE, KEY_PRESS_SIGNAL};

pub(crate) static PERIPHERAL_BATTERY_UPDATE: Signal<crate::RawMutex, BatteryUpdate> = Signal::new();
pub(crate) static PERIPHERAL_BATTERY_KEY_PRESS_SIGNAL: Signal<crate::RawMutex, u32> = Signal::new();

/// Battery service
#[gatt_service(uuid = service::BATTERY)]
pub(crate) struct BatteryService {
    /// Battery Level
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    // CPF namespace description 0x0106 = "main": tells hosts (e.g. macOS) to
    // treat this as the primary battery when several BAS instances exist.
    #[descriptor(uuid = descriptors::CHARACTERISTIC_PRESENTATION_FORMAT, read, value = [0x04, 0x00, 0xad, 0x27, 0x01, 0x06, 0x01])]
    #[descriptor(uuid = descriptors::CHARACTERISTIC_USER_DESCRIPTION, read, value = "Left")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify)]
    pub(crate) level: u8,
}

/// Auxiliary battery service for the right split half.
#[gatt_service(uuid = service::BATTERY)]
pub(crate) struct PeripheralBatteryService {
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(uuid = descriptors::CHARACTERISTIC_PRESENTATION_FORMAT, read, value = [0x04, 0x00, 0xad, 0x27, 0x01, 0x08, 0x01])]
    #[descriptor(uuid = descriptors::CHARACTERISTIC_USER_DESCRIPTION, read, value = "Right")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify)]
    pub(crate) level: u8,
}

pub(crate) fn update_peripheral_battery(level: u8) {
    PERIPHERAL_BATTERY_UPDATE.signal(BatteryUpdate {
        state: BatteryState::Normal(level),
        key_press_sequence: KEY_PRESS_SEQUENCE.load(Ordering::Acquire),
    });
}

pub(crate) struct BleBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    pub(crate) battery_level: Characteristic<u8>,
    pub(crate) conn: &'conn GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'conn, P: PacketPool> BleBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.battery_service.level,
            conn,
        }
    }
}

pub(crate) struct BlePeripheralBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    pub(crate) battery_level: Characteristic<u8>,
    pub(crate) conn: &'conn GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'conn, P: PacketPool> BlePeripheralBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.peripheral_battery_service.level,
            conn,
        }
    }
}

impl<P: PacketPool> BlePeripheralBatteryServer<'_, '_, '_, P> {
    pub(crate) async fn run(&mut self) {
        Timer::after_secs(2).await;

        loop {
            let BatteryUpdate {
                state,
                key_press_sequence: battery_sequence,
            } = PERIPHERAL_BATTERY_UPDATE.wait().await;
            wait_for_key_press_after(battery_sequence, &PERIPHERAL_BATTERY_KEY_PRESS_SIGNAL).await;

            if let BatteryState::Normal(level) = state
                && let Err(e) = self.battery_level.notify(self.conn, &level).await
            {
                error!("Failed to notify peripheral battery level: {:?}", e);
            }
        }
    }
}

impl<P: PacketPool> BleBatteryServer<'_, '_, '_, P> {
    pub(crate) async fn run(&mut self) {
        // Wait 2 seconds, ensure that gatt server has been started
        Timer::after_secs(2).await;

        // Report the battery level.
        loop {
            let battery_state = self.wait_until_battery_state_available().await;
            if let BatteryState::Normal(level) = battery_state
                && let Err(e) = self.battery_level.notify(self.conn, &level).await
            {
                error!("Failed to notify battery level: {:?}", e);
            }
        }
    }

    /// Wait until the battery state is available.
    /// To avoid unexpected wakeup, before reporting battery level, all conditions should be satisfied:
    ///
    /// 1. There's a battery state update
    /// 2. A new key press occurs after that update
    ///
    /// Battery notifications are never sent on a timer. macOS may skip the HID
    /// suspend command when the display turns off just before system sleep, so
    /// relying on the sleep flag alone can let a periodic battery notification
    /// wake the machine.
    async fn wait_until_battery_state_available(&mut self) -> BatteryState {
        let BatteryUpdate {
            state,
            key_press_sequence: battery_sequence,
        } = BATTERY_UPDATE.wait().await;

        wait_for_key_press_after(battery_sequence, &KEY_PRESS_SIGNAL).await;

        state
    }
}

async fn wait_for_key_press_after(
    battery_sequence: u32,
    key_press_signal: &Signal<crate::RawMutex, u32>,
) {
    // Wait for a press that happened after this battery update. Sequence
    // comparison avoids losing a press between reading the update and waiting.
    loop {
        let current_sequence = KEY_PRESS_SEQUENCE.load(Ordering::Acquire);
        if is_sequence_after(current_sequence, battery_sequence) {
            break;
        }

        let key_sequence = key_press_signal.wait().await;
        if is_sequence_after(key_sequence, battery_sequence) {
            break;
        }
    }
}

fn is_sequence_after(candidate: u32, baseline: u32) -> bool {
    let distance = candidate.wrapping_sub(baseline);
    distance != 0 && distance < (1 << 31)
}
