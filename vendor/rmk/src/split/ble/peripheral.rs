#[cfg(feature = "subrating")]
use bt_hci::{cmd::le::LeSetHostFeature, controller::ControllerCmdSync};
use embassy_futures::join::join;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use super::{GattSplitMessage, SplitMessage};
use crate::ble::adv::{Adv, advertise};
use crate::event::{
    CentralConnectedEvent, KeyboardEvent, PeripheralReconnectByKeyEvent, SleepStateEvent, SubscribableEvent,
    publish_event,
};
use crate::split::driver::{SplitDriverError, SplitReader, SplitWriter};
use crate::split::peripheral::SplitPeripheral;
use crate::state::update_status;

const IDLE_RECONNECT_PAUSE: Duration = Duration::from_secs(20);
// 左半连着主机休眠时每隔 5 秒尝试 1 秒，广播窗口需覆盖一次完整尝试周期。
const IDLE_RECONNECT_WINDOW: Duration = Duration::from_secs(8);

/// Gatt service used in split peripheral to send split message to central
#[gatt_service(uuid = "4dd5fbaa-18e5-4b07-bf0a-353698659946")]
pub(crate) struct SplitBleService {
    #[characteristic(uuid = "0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3", read, notify, indicate)]
    pub(crate) message_to_central: GattSplitMessage,

    #[characteristic(uuid = "4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c", write_without_response, read, notify)]
    pub(crate) message_to_peripheral: GattSplitMessage,
}

/// Gatt server in split peripheral
#[gatt_server]
pub(crate) struct BleSplitPeripheralServer {
    pub(crate) service: SplitBleService,
}

/// BLE driver for split peripheral
pub(crate) struct BleSplitPeripheralDriver<'stack, 'server, 'c, P: PacketPool> {
    message_to_peripheral: Characteristic<GattSplitMessage>,
    message_to_central: Characteristic<GattSplitMessage>,
    conn: &'c GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'c, P: PacketPool> BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    pub(crate) fn new(server: &'server BleSplitPeripheralServer, conn: &'c GattConnection<'stack, 'server, P>) -> Self {
        Self {
            message_to_central: server.service.message_to_central.clone(),
            message_to_peripheral: server.service.message_to_peripheral.clone(),
            conn,
        }
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitReader for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        let message = loop {
            match self.conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    error!("Disconnected from central: {:?}", reason);
                    update_status(|c| *c = ConnectionStatus::new());
                    return Err(SplitDriverError::Disconnected);
                }
                GattConnectionEvent::Gatt { event: gatt_event } => {
                    match &gatt_event {
                        GattEvent::Read(event) => {
                            info!("Gatt read event: {:?}", event.handle());
                        }
                        GattEvent::Write(event) => {
                            // Write to peripheral
                            if event.handle() == self.message_to_peripheral.handle {
                                let parsed = event.with_data(|_, data| {
                                    trace!("Got message from central: {:?}", data);
                                    postcard::from_bytes::<SplitMessage>(data)
                                });
                                match parsed {
                                    Ok(message) => {
                                        trace!("Message from central: {:?}", message);
                                        break message;
                                    }
                                    Err(e) => error!("Postcard deserialize split message error: {}", e),
                                }
                            } else {
                                info!("Gatt write other event: {:?}", event.handle());
                            }
                        }
                        _ => debug!("Other gatt event"),
                    };
                    match gatt_event.accept() {
                        Ok(r) => r.send().await,
                        Err(e) => warn!("[gatt] error sending response: {:?}", e),
                    }
                }
                GattConnectionEvent::ConnectionParamsUpdated {
                    conn_interval,
                    peripheral_latency,
                    supervision_timeout,
                } => info!(
                    "[split] params updated: interval {:?}us, latency {:?}, timeout {:?}ms",
                    conn_interval.as_micros(),
                    peripheral_latency,
                    supervision_timeout.as_millis()
                ),
                GattConnectionEvent::SubratingParamsUpdated {
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout,
                } => info!(
                    "[split] subrating updated: subrate {:?}, latency {:?}, continuation {:?}, timeout {:?}ms",
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout.as_millis()
                ),
                GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                    info!("[split] PHY updated: {:?}, {:?}", tx_phy, rx_phy)
                }
                _ => (),
            }
        };
        Ok(message)
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitWriter for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let gatt_msg = GattSplitMessage::try_from(message)?;
        debug!("Writing split message to central: {:?}", message);
        self.message_to_central
            .notify(self.conn, &gatt_msg, true)
            .await
            .map_err(|e| {
                error!("BLE notify error: {:?}", e);
                SplitDriverError::BleError(1)
            })?;
        Ok(gatt_msg.len)
    }
}

/// Let the controller accept the central's subrate requests on the split link.
///
/// Must run concurrently with `ble_task()` (whose runner serves the HCI command)
/// and before any advertising, since the flag only applies to links opened after
/// it is set.
#[cfg(feature = "subrating")]
async fn init_subrating_host_feature<C: Controller + ControllerCmdSync<LeSetHostFeature>>(
    stack: &Stack<'_, C, impl PacketPool>,
) {
    const CONN_SUBRATING_HOST_BIT: u8 = 38;
    let cmd = LeSetHostFeature::new(CONN_SUBRATING_HOST_BIT, 1);
    if let Err(e) = stack.command(cmd).await {
        error!("[split_peri] error setting subrating host feature flag: {:?}", e);
    }
}

/// Initialize and run the nRF peripheral keyboard service via BLE.
///
/// # Arguments
///
/// * `id` - The id of the peripheral
/// * `central_addr` - The address of the central
/// * `stack` - The stack to use
pub async fn initialize_nrf_ble_split_peripheral_and_run<
    'b,
    's: 'b,
    #[cfg(feature = "subrating")] C: Controller + ControllerCmdSync<LeSetHostFeature>,
    #[cfg(not(feature = "subrating"))] C: Controller,
>(
    id: usize,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
) {
    publish_event(CentralConnectedEvent { connected: false });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // First, read central address from storage
    let mut central_addr = crate::storage::read_peer_address(0)
        .await
        .filter(|a| a.is_valid)
        .map(|a| a.address);

    let peri_task = async {
        // Set subrating host support before any advertising/connecting
        #[cfg(feature = "subrating")]
        init_subrating_host_feature(stack).await;

        let server = BleSplitPeripheralServer::new_default("rmk").unwrap();
        loop {
            update_status(|c| *c = ConnectionStatus::new());
            publish_event(CentralConnectedEvent { connected: false });
            publish_event(SleepStateEvent::new(false));
            let connection = match split_peripheral_advertise(id, central_addr, &mut peripheral, &server).await {
                Err(BleHostError::BleHost(Error::Timeout)) => {
                    publish_event(SleepStateEvent::new(true));
                    let idle_attempt = async || {
                        let addr = Address::random(central_addr?);
                        let result = async {
                            peripheral.set_filter_accept_list(&[addr]).await?;
                            advertise(
                                &mut peripheral,
                                &server.server,
                                Adv::IdleSplitPeripheral { id: id as u8 },
                                IDLE_RECONNECT_WINDOW,
                            )
                            .await
                        }
                        .await;
                        match result {
                            Ok(conn) => Some(conn),
                            Err(BleHostError::BleHost(Error::Timeout)) => None,
                            Err(err) => {
                                #[cfg(feature = "defmt")]
                                let err = defmt::Debug2Format(&err);
                                warn!("Idle split advertising failed: {:?}", err);
                                None
                            }
                        }
                    };
                    let Some(conn) = wait_for_idle_reconnect(central_addr.is_some(), idle_attempt).await else {
                        publish_event(PeripheralReconnectByKeyEvent);
                        continue;
                    };
                    publish_event(SleepStateEvent::new(false));
                    Ok(conn)
                }
                result => result,
            };
            match connection {
                Ok(conn) => {
                    info!("Connected to the central");
                    publish_event(CentralConnectedEvent { connected: true });
                    let mut peripheral = SplitPeripheral::new(BleSplitPeripheralDriver::new(&server, &conn));
                    let new_addr = conn.raw().peer_address().addr.into_inner();
                    if central_addr != Some(new_addr) {
                        info!("Saving central address to storage");
                        if crate::storage::write_peer_address(PeerAddress {
                            peer_id: 0,
                            is_valid: true,
                            address: new_addr,
                        })
                        .await
                        {
                            central_addr = Some(new_addr);
                        }
                    }
                    if let Err(e) = peripheral.run().await {
                        error!("Split peripheral session failed: {:?}", e);
                    }
                    // 发送失败也必须主动断开，让中央侧释放按键并允许重新连接。
                    conn.raw().disconnect();
                    info!("Disconnected from the central");
                }
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(500).await;
                    continue;
                }
            };
        }
    };

    join(crate::ble::ble_task(runner, &crate::ble::NoopHandler), peri_task).await;
}

async fn wait_for_idle_reconnect<T>(has_saved_peer: bool, mut attempt: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    let mut keys = KeyboardEvent::subscriber();
    let pressed = async { while !keys.next_message_pure().await.pressed {} };
    let reconnect = async {
        if !has_saved_peer {
            return core::future::pending().await;
        }
        loop {
            Timer::after(IDLE_RECONNECT_PAUSE).await;
            if let Some(conn) = attempt().await {
                return conn;
            }
        }
    };
    match select(reconnect, pressed).await {
        Either::First(conn) => Some(conn),
        Either::Second(()) => None,
    }
}

/// Reconnect to the saved central, falling back to seeking any central when it
/// does not answer.
async fn split_peripheral_advertise<'a, 'b, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    if let Some(addr) = central_addr {
        let directed = Adv::Directed(Address::random(addr));
        match advertise(peripheral, &server.server, directed, Duration::from_secs(10)).await {
            Err(BleHostError::BleHost(Error::Timeout)) => warn!("[adv] Try update central_addr"),
            result => return result,
        }
    }
    let seeking = Adv::SplitPeripheral { id: id as u8 };
    advertise(peripheral, &server.server, seeking, Duration::from_secs(300)).await
}

#[cfg(test)]
mod tests {
    use embassy_time::Instant;

    use super::*;
    use crate::event::{AsyncEventPublisher, AsyncPublishableEvent};
    use crate::test_support::test_block_on;

    #[test]
    fn idle_reconnect_retries_after_each_pause() {
        test_block_on(async {
            let start = Instant::now();
            let mut attempts = 0;
            let connection = wait_for_idle_reconnect(true, async || {
                attempts += 1;
                if attempts == 1 {
                    assert!(start.elapsed() >= IDLE_RECONNECT_PAUSE);
                    Timer::after(IDLE_RECONNECT_WINDOW).await;
                    return None;
                }
                Some(attempts)
            })
            .await;
            assert_eq!(connection, Some(2));
            assert!(start.elapsed() >= IDLE_RECONNECT_PAUSE * 2 + IDLE_RECONNECT_WINDOW);
        });
    }

    #[test]
    fn idle_reconnect_wakes_on_press_with_or_without_saved_peer() {
        test_block_on(async {
            let sender = KeyboardEvent::publisher_async().unwrap();
            for has_saved_peer in [false, true] {
                let mut attempted = false;
                let start = Instant::now();
                let result = select(
                    wait_for_idle_reconnect(has_saved_peer, async || {
                        attempted = true;
                        core::future::pending::<Option<()>>().await
                    }),
                    async {
                        Timer::after(IDLE_RECONNECT_PAUSE + Duration::from_secs(1)).await;
                        sender.publish_async(KeyboardEvent::key(0, 0, false)).await;
                        Timer::after_secs(1).await;
                        sender.publish_async(KeyboardEvent::key(0, 0, true)).await;
                        core::future::pending::<()>().await;
                    },
                )
                .await;
                assert!(matches!(result, Either::First(None)));
                assert!(start.elapsed() >= IDLE_RECONNECT_PAUSE + Duration::from_secs(2));
                assert_eq!(attempted, has_saved_peer);
            }
        });
    }
}
