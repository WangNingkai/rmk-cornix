use core::sync::atomic::Ordering;

use ssmarshal::serialize;
use trouble_host::prelude::*;
use usbd_hid::descriptor::SerializedDescriptor;

use super::battery_service::{BatteryService, PeripheralBatteryService};
use super::device_info::DeviceConfigrmationService;
#[cfg(feature = "host")]
use super::host_service::HostService;
use crate::ble::SLEEPING_STATE;
use crate::channel::KEYBOARD_REPORT_CHANNEL;
use crate::descriptor::{CompositeReport, CompositeReportType, KeyboardReport};
use crate::hid::{HidError, HidWriterTrait, Report, RunnableHidWriter};

// Used for saving the CCCD table
pub(crate) const CCCD_TABLE_SIZE: usize = _CCCD_TABLE_SIZE;

// GATT Server definition
// NOTE: ideally we would conditionally add the `via_service` member, based on the
// `vial` feature flag. But when doing that, rust still compiles the member as if
// the flag was on, for some reason. I suspect it might have something to do with
// the `gatt_server` macro, but I'm not sure. So we need 2 versions of the Server
// struct, one with vial support, and one without.
#[cfg(feature = "host")]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) peripheral_battery_service: PeripheralBatteryService,
    pub(crate) hid_service: HidService,
    pub(crate) host_service: HostService,
    pub(crate) composite_service: CompositeService,
    pub(crate) device_config_service: DeviceConfigrmationService,
}

#[cfg(not(feature = "host"))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) peripheral_battery_service: PeripheralBatteryService,
    pub(crate) hid_service: HidService,
    pub(crate) composite_service: CompositeService,
    pub(crate) device_config_service: DeviceConfigrmationService,
}

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct HidService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = KeyboardReport::desc().try_into().expect("Failed to convert KeyboardReport to [u8; 67]"))]
    pub(crate) report_map: [u8; 67],
    #[characteristic(uuid = "2a4c", write_without_response)]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    pub(crate) protocol_mode: u8,
    #[descriptor(uuid = "2908", read, value = [0u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) input_keyboard: [u8; 8],
    #[descriptor(uuid = "2908", read, value = [0u8, 2u8])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response)]
    pub(crate) output_keyboard: [u8; 1],
}

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct CompositeService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = CompositeReport::desc().try_into().expect("Failed to convert CompositeReport to [u8; 111]"))]
    pub(crate) report_map: [u8; 111],
    #[characteristic(uuid = "2a4c", write_without_response)]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    pub(crate) protocol_mode: u8,
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Mouse as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) mouse_report: [u8; 5],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Media as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) media_report: [u8; 2],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::System as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) system_report: [u8; 1],
}

pub(crate) struct BleHidServer<'stack, 'server, 'conn, P: PacketPool> {
    pub(crate) input_keyboard: Characteristic<[u8; 8]>,
    pub(crate) mouse_report: Characteristic<[u8; 5]>,
    pub(crate) media_report: Characteristic<[u8; 2]>,
    pub(crate) system_report: Characteristic<[u8; 1]>,
    pub(crate) conn: &'conn GattConnection<'stack, 'server, P>,
    pending_sleep_key_release: bool,
}

fn should_notify_keyboard_report(
    pending_sleep_key_release: &mut bool,
    report: &KeyboardReport,
    sleeping: bool,
) -> bool {
    let has_key_activity = report.modifier != 0 || report.keycodes.iter().any(|&key| key != 0);

    if !sleeping {
        // Remember an active report so its release can still be delivered if
        // the keyboard enters sleep while the key remains held.
        *pending_sleep_key_release = has_key_activity;
        return true;
    }

    if has_key_activity {
        *pending_sleep_key_release = true;
        return true;
    }

    if *pending_sleep_key_release {
        *pending_sleep_key_release = false;
        return true;
    }

    false
}

impl<'stack, 'server, 'conn, P: PacketPool> BleHidServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            input_keyboard: server.hid_service.input_keyboard,
            mouse_report: server.composite_service.mouse_report,
            media_report: server.composite_service.media_report,
            system_report: server.composite_service.system_report,
            conn,
            pending_sleep_key_release: false,
        }
    }

    fn should_notify_keyboard_report(&mut self, report: &KeyboardReport) -> bool {
        should_notify_keyboard_report(
            &mut self.pending_sleep_key_release,
            report,
            SLEEPING_STATE.load(Ordering::Acquire),
        )
    }
}

impl<P: PacketPool> HidWriterTrait for BleHidServer<'_, '_, '_, P> {
    type ReportType = Report;

    async fn write_report(&mut self, report: Self::ReportType) -> Result<usize, HidError> {
        match report {
            Report::KeyboardReport(keyboard_report) => {
                if !self.should_notify_keyboard_report(&keyboard_report) {
                    debug!("Suppressing empty keyboard report while sleeping");
                    return Ok(0);
                }
                let mut buf = [0u8; 8];
                let n = serialize(&mut buf, &keyboard_report).map_err(|_| HidError::ReportSerializeError)?;
                self.input_keyboard.notify(self.conn, &buf).await.map_err(|e| {
                    error!("Failed to notify keyboard report: {:?}", e);
                    HidError::BleError
                })?;
                Ok(n)
            }
            Report::MouseReport(mouse_report) => {
                let mut buf = [0u8; 5];
                let n = serialize(&mut buf, &mouse_report).map_err(|_| HidError::ReportSerializeError)?;
                self.mouse_report.notify(self.conn, &buf).await.map_err(|e| {
                    error!("Failed to notify mouse report: {:?}", e);
                    HidError::BleError
                })?;
                Ok(n)
            }
            Report::MediaKeyboardReport(media_keyboard_report) => {
                let mut buf = [0u8; 2];
                let n = serialize(&mut buf, &media_keyboard_report).map_err(|_| HidError::ReportSerializeError)?;
                self.media_report.notify(self.conn, &buf).await.map_err(|e| {
                    error!("Failed to notify media report: {:?}", e);
                    HidError::BleError
                })?;
                Ok(n)
            }
            Report::SystemControlReport(system_control_report) => {
                let mut buf = [0u8; 1];
                let n = serialize(&mut buf, &system_control_report).map_err(|_| HidError::ReportSerializeError)?;
                self.system_report.notify(self.conn, &buf).await.map_err(|e| {
                    error!("Failed to notify system report: {:?}", e);
                    HidError::BleError
                })?;
                Ok(n)
            }
        }
    }
}

impl<P: PacketPool> RunnableHidWriter for BleHidServer<'_, '_, '_, P> {
    async fn get_report(&mut self) -> Self::ReportType {
        KEYBOARD_REPORT_CHANNEL.receive().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_key_held_across_sleep() {
        let mut pending_release = false;
        let pressed = KeyboardReport {
            keycodes: [4, 0, 0, 0, 0, 0],
            ..Default::default()
        };
        let released = KeyboardReport::default();

        assert!(should_notify_keyboard_report(
            &mut pending_release,
            &pressed,
            false
        ));
        assert!(pending_release);

        assert!(should_notify_keyboard_report(
            &mut pending_release,
            &released,
            true
        ));
        assert!(!pending_release);

        assert!(!should_notify_keyboard_report(
            &mut pending_release,
            &released,
            true
        ));
    }
}
