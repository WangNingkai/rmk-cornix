use core::fmt::Debug;

use embassy_embedded_hal::adapter::BlockingAsync;
#[cfg(feature = "vial")]
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_storage::nor_flash::NorFlash;
use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use postcard::experimental::max_size::MaxSize;
use rmk_types::connection::ConnectionType;
use rmk_types::morse::MorseProfile;
use sequential_storage::Error as SSError;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{Key, MapConfig, MapStorage, PostcardValue, SerializationError};
#[cfg(feature = "host")]
use {
    crate::{MACRO_SPACE_SIZE, keyboard::combo::ComboConfig},
    rmk_types::action::{EncoderAction, KeyAction},
    rmk_types::fork::Fork,
    rmk_types::morse::Morse,
};

#[cfg(feature = "_ble")]
use crate::ble::profile::ProfileInfo;
use crate::boot::reboot_keyboard;
use crate::channel::FLASH_CHANNEL;
use crate::config;
use crate::config::StorageConfig;
#[cfg(all(feature = "_ble", feature = "split"))]
use crate::split::ble::PeerAddress;

// 首字节区分旧版布尔标记，格式不兼容时提高第二字节的版本号。
const STORAGE_FORMAT: [u8; 2] = [b'C', 1];

/// Signal to synchronize the flash operation status, usually used outside of the flash task.
/// True if the flash operation is finished correctly, false if the flash operation is finished with error.
pub(crate) static FLASH_OPERATION_FINISHED: Signal<crate::RawMutex, bool> = Signal::new();

#[cfg(feature = "vial")]
static VIAL_WRITE_SEQUENCE: Mutex<crate::RawMutex, u32> = Mutex::new(0);
#[cfg(feature = "vial")]
static VIAL_WRITE_RESULT: Signal<crate::RawMutex, (u32, bool)> = Signal::new();

#[cfg(feature = "vial")]
pub(crate) async fn save_vial_keymap(write: VialKeymapWrite) -> bool {
    // USB 与蓝牙共用一个写入回执。会话取消后，下一条请求跳过旧回执。
    let mut sequence = VIAL_WRITE_SEQUENCE.lock().await;
    *sequence = sequence.wrapping_add(1);
    FLASH_CHANNEL
        .send(FlashOperationMessage::VialKeymap { id: *sequence, write })
        .await;
    loop {
        let (id, saved) = VIAL_WRITE_RESULT.wait().await;
        if id == *sequence {
            return saved;
        }
    }
}

#[cfg(feature = "vial")]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum VialKeymapWrite {
    Key {
        layer: u8,
        row: u8,
        col: u8,
        action: KeyAction,
    },
    Encoder {
        layer: u8,
        idx: u8,
        clockwise: bool,
        action: KeyAction,
    },
}

// Request/response over `FLASH_CHANNEL`. One `Signal` per read variant; the
// storage task fires the matching one once it has the result.
#[cfg(feature = "_ble")]
static BOND_INFO_RESPONSE: Signal<crate::RawMutex, Option<ProfileInfo>> = Signal::new();
#[cfg(all(feature = "_ble", feature = "split"))]
static PEER_ADDRESS_RESPONSE: Signal<crate::RawMutex, Option<PeerAddress>> = Signal::new();
#[cfg(feature = "_ble")]
static CONNECTION_TYPE_RESPONSE: Signal<crate::RawMutex, Option<ConnectionType>> = Signal::new();
#[cfg(feature = "_ble")]
static ACTIVE_BLE_PROFILE_RESPONSE: Signal<crate::RawMutex, Option<u8>> = Signal::new();

#[cfg(feature = "_ble")]
async fn request_read<T: Send>(msg: FlashOperationMessage, response: &Signal<crate::RawMutex, T>) -> T {
    response.reset();
    FLASH_CHANNEL.send(msg).await;
    response.wait().await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_bond_info(slot_num: u8) -> Option<ProfileInfo> {
    request_read(FlashOperationMessage::ReadBleBondInfo(slot_num), &BOND_INFO_RESPONSE).await
}

#[cfg(all(feature = "_ble", feature = "split"))]
pub(crate) async fn read_peer_address(peer_id: u8) -> Option<PeerAddress> {
    request_read(FlashOperationMessage::ReadPeerAddress(peer_id), &PEER_ADDRESS_RESPONSE).await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_connection_type() -> Option<ConnectionType> {
    request_read(FlashOperationMessage::ReadConnectionType, &CONNECTION_TYPE_RESPONSE).await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_active_ble_profile() -> Option<u8> {
    request_read(
        FlashOperationMessage::ReadActiveBleProfile,
        &ACTIVE_BLE_PROFILE_RESPONSE,
    )
    .await
}

/// Send a peer address to be persisted; wait for the storage task to finish.
/// Returns `true` if the write completed successfully.
#[cfg(all(feature = "_ble", feature = "split"))]
pub(crate) async fn write_peer_address(addr: PeerAddress) -> bool {
    FLASH_OPERATION_FINISHED.reset();
    FLASH_CHANNEL.send(FlashOperationMessage::PeerAddress(addr)).await;
    FLASH_OPERATION_FINISHED.wait().await
}

// Message send from other tasks, which will do saving or clearing operation
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum FlashOperationMessage {
    #[cfg(feature = "_ble")]
    // BLE profile info to be saved
    ProfileInfo(ProfileInfo),
    #[cfg(feature = "_ble")]
    // Current active BLE profile number
    ActiveBleProfile(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    // Peer address
    PeerAddress(PeerAddress),
    // Clear the storage
    Reset,
    // Clear the layout info
    ResetLayout,
    #[cfg(feature = "_ble")]
    // Clear info of given slot number
    ClearSlot(u8),
    // Layout option
    LayoutOptions(u32),
    // Default layer number
    DefaultLayer(u8),
    #[cfg(feature = "host")]
    MacroData([u8; MACRO_SPACE_SIZE]),
    #[cfg(feature = "rynk")]
    KeymapKey {
        layer: u8,
        row: u8,
        col: u8,
        action: KeyAction,
    },
    #[cfg(feature = "rynk")]
    Encoder {
        layer: u8,
        idx: u8,
        action: EncoderAction,
    },
    #[cfg(feature = "vial")]
    VialKeymap {
        id: u32,
        write: VialKeymapWrite,
    },
    #[cfg(feature = "host")]
    Combo {
        idx: u8,
        config: ComboConfig,
    },
    #[cfg(feature = "host")]
    Fork {
        idx: u8,
        fork: Fork,
    },
    #[cfg(feature = "host")]
    Morse {
        idx: u8,
        morse: Morse,
    },
    // Current saved connection type
    ConnectionType(ConnectionType),
    // Timeout time for combos
    ComboTimeout(u16),
    // Timeout time for one-shot keys
    OneShotTimeout(u16),
    // Interval for tap actions
    TapInterval(u16),
    // Interval for tapping capslock
    TapCapslockInterval(u16),
    // The prior-idle-time in ms used for in flow tap
    PriorIdleTime(u16),
    // Default morse profile containing all morse/tap-hold settings (mode, timeouts, unilateral_tap)
    MorseDefaultProfile(MorseProfile),
    #[cfg(feature = "rynk")]
    // The whole behavior config in one message (Rynk's SetBehaviorConfig carries
    // every field, so one store beats six read-modify-write cycles)
    BehaviorConfig(BehaviorConfig),
    #[cfg(feature = "_ble")]
    // Read bond info for the given slot; storage task replies via `BOND_INFO_RESPONSE`.
    ReadBleBondInfo(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    // Read peer address for the given peer id; storage task replies via `PEER_ADDRESS_RESPONSE`.
    ReadPeerAddress(u8),
    #[cfg(feature = "_ble")]
    // Read the persisted `ConnectionType`; storage task replies via `CONNECTION_TYPE_RESPONSE`.
    ReadConnectionType,
    #[cfg(feature = "_ble")]
    // Read the persisted active BLE profile number; storage task replies via `ACTIVE_BLE_PROFILE_RESPONSE`.
    ReadActiveBleProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StorageKey {
    FormatVersion,
    LayoutConfig,
    BehaviorConfig,
    ConnectionType,
    #[cfg(feature = "host")]
    MacroData,
    #[cfg(feature = "host")]
    Keymap {
        layer: u8,
        row: u8,
        col: u8,
    },
    #[cfg(feature = "host")]
    Encoder {
        layer: u8,
        idx: u8,
    },
    #[cfg(feature = "host")]
    Combo(u8),
    #[cfg(feature = "host")]
    Fork(u8),
    #[cfg(feature = "host")]
    Morse(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    PeerAddress(u8),
    #[cfg(feature = "_ble")]
    ActiveBleProfile,
    #[cfg(feature = "_ble")]
    BondInfo(u8),
}

impl StorageKey {
    #[cfg(feature = "host")]
    pub(crate) const fn keymap(layer: u8, row: u8, col: u8) -> Self {
        Self::Keymap { layer, row, col }
    }

    #[cfg(feature = "_ble")]
    pub(crate) const fn bond_info(slot_num: u8) -> Self {
        Self::BondInfo(slot_num)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn combo(idx: u8) -> Self {
        Self::Combo(idx)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn encoder(idx: u8, layer: u8) -> Self {
        Self::Encoder { layer, idx }
    }

    #[cfg(feature = "host")]
    pub(crate) const fn fork(idx: u8) -> Self {
        Self::Fork(idx)
    }

    #[cfg(all(feature = "_ble", feature = "split"))]
    pub(crate) const fn peer_address(peer_id: u8) -> Self {
        Self::PeerAddress(peer_id)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn morse(idx: u8) -> Self {
        Self::Morse(idx)
    }
}

impl Key for StorageKey {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        postcard::to_slice(self, buffer)
            .map(|used| used.len())
            .map_err(Into::into)
    }

    fn deserialize_from(buffer: &[u8]) -> Result<(Self, usize), SerializationError> {
        let (key, rest): (Self, &[u8]) = postcard::take_from_bytes(buffer).map_err(SerializationError::from)?;
        Ok((key, buffer.len() - rest.len()))
    }

    fn get_len(buffer: &[u8]) -> Result<usize, SerializationError> {
        Self::deserialize_from(buffer).map(|(_, len)| len)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum StorageData {
    FormatVersion([u8; 2]),
    LayoutConfig(LayoutConfig),
    BehaviorConfig(BehaviorConfig),
    ConnectionType(ConnectionType),
    #[cfg(feature = "host")]
    MacroData(#[serde(with = "crate::host::storage::macro_bytes_serde")] [u8; MACRO_SPACE_SIZE]),
    #[cfg(feature = "host")]
    KeyAction(KeyAction),
    #[cfg(feature = "host")]
    EncoderAction(EncoderAction),
    #[cfg(feature = "host")]
    Combo(ComboConfig),
    #[cfg(feature = "host")]
    Fork(Fork),
    #[cfg(feature = "host")]
    Morse(Morse),
    #[cfg(all(feature = "_ble", feature = "split"))]
    PeerAddress(PeerAddress),
    #[cfg(feature = "_ble")]
    BondInfo(ProfileInfo),
    #[cfg(feature = "_ble")]
    ActiveBleProfile(u8),
}

impl<'a> PostcardValue<'a> for StorageData {}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LayoutConfig {
    pub(crate) default_layer: u8,
    pub(crate) layout_option: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct BehaviorConfig {
    // The prior-idle-time in ms used for in flow tap
    pub(crate) prior_idle_time: u16,
    // Default morse profile containing mode, timeouts, and unilateral_tap settings
    pub(crate) morse_default_profile: MorseProfile,

    // Timeout time for combos
    pub(crate) combo_timeout: u16,
    // Timeout time for one-shot keys
    pub(crate) one_shot_timeout: u16,
    // Interval for tap actions
    pub(crate) tap_interval: u16,
    // Interval for tapping capslock.
    // macOS has special processing of capslock, when tapping capslock, the tap interval should be another value
    pub(crate) tap_capslock_interval: u16,
}

impl From<LayoutConfig> for StorageData {
    fn from(config: LayoutConfig) -> Self {
        Self::LayoutConfig(config)
    }
}

impl From<&config::BehaviorConfig> for StorageData {
    fn from(behavior: &config::BehaviorConfig) -> Self {
        // Note: default_layer persists via LayoutConfig (restored in read_keymap), not this struct.
        Self::BehaviorConfig(BehaviorConfig {
            prior_idle_time: behavior.morse.prior_idle_time.as_millis() as u16,
            morse_default_profile: behavior.morse.default_profile,
            combo_timeout: behavior.combo.timeout.as_millis() as u16,
            one_shot_timeout: behavior.one_shot.timeout.as_millis() as u16,
            tap_interval: behavior.tap.tap_interval,
            tap_capslock_interval: behavior.tap.tap_capslock_interval,
        })
    }
}

pub fn async_flash_wrapper<F: NorFlash>(flash: F) -> BlockingAsync<F> {
    embassy_embedded_hal::adapter::BlockingAsync::new(flash)
}

/// Storage for the firmwares that hold no keymap of their own — a split
/// peripheral and a dongle. Both still persist their BLE bonds, which the
/// profile manager loads over `FLASH_CHANNEL`.
#[cfg(any(feature = "split", feature = "dongle"))]
pub async fn new_storage_without_keymap<F: AsyncNorFlash>(
    flash: F,
    storage_config: StorageConfig,
) -> Storage<F, 0, 0, 0, 0> {
    Storage::<F, 0, 0, 0, 0>::new(
        flash,
        #[cfg(feature = "host")]
        &[],
        #[cfg(feature = "host")]
        &None,
        &storage_config,
        &config::BehaviorConfig::default(),
    )
    .await
}

type StorageCache = Cache<Uncached, Uncached, Uncached, StorageKey>;

pub struct Storage<
    F: AsyncNorFlash,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize = 0,
> {
    pub(crate) flash: MapStorage<StorageKey, F, StorageCache>,
    pub(crate) buffer: [u8; get_buffer_size()],
    pub(crate) ready: bool,
}

/// Read out storage config, update and then save back.
/// This macro applies to only some of the configs.
macro_rules! update_storage_field {
    ($f: expr, $buf: expr, $key:ident, $field:ident) => {{
        let key = StorageKey::$key;
        if let Ok(Some(StorageData::$key(mut saved))) = $f.fetch_item($buf, &key).await {
            saved.$field = $field;
            $f.store_item($buf, &key, &StorageData::$key(saved)).await
        } else {
            Ok(())
        }
    }};
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn fetch_data(&mut self, key: StorageKey) -> Option<StorageData> {
        if !self.ready {
            return None;
        }
        match self.flash.fetch_item(&mut self.buffer, &key).await {
            Ok(data) => data,
            Err(e) => {
                print_storage_error::<F>(e);
                None
            }
        }
    }

    async fn store_data(&mut self, key: StorageKey, data: &StorageData) -> Result<(), SSError<F::Error>> {
        self.flash.store_item(&mut self.buffer, &key, data).await
    }

    pub async fn new(
        flash: F,
        #[cfg(feature = "host")] keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        #[cfg(feature = "host")] encoder_map: &Option<&mut [[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        storage_config: &StorageConfig,
        behavior_config: &config::BehaviorConfig,
    ) -> Self {
        // Check storage setting
        assert!(
            storage_config.num_sectors >= 2,
            "Number of used sector for storage must larger than 1"
        );

        // If config.start_addr == 0:
        // - For nRF chips: use sectors starting at 0x0006_0000
        // - For other chips: use the last `num_sectors` sectors
        // Otherwise, use storage config setting
        // When DFU is active the storage partition already sits at the correct
        // offset — the _nrf_ble special case (0x60000) only applies without DFU.
        #[cfg(all(feature = "_nrf_ble", not(any(feature = "dfu_rp", feature = "dfu_nrf"))))]
        let start_addr = if storage_config.start_addr == 0 {
            0x0006_0000
        } else {
            storage_config.start_addr
        };

        #[cfg(not(all(feature = "_nrf_ble", not(any(feature = "dfu_rp", feature = "dfu_nrf")))))]
        let start_addr = storage_config.start_addr;
        // Check storage setting
        info!(
            "Flash capacity {} KB, RMK use {} KB({} sectors) starting from 0x{:X} as storage",
            flash.capacity() / 1024,
            (F::ERASE_SIZE * storage_config.num_sectors as usize) / 1024,
            storage_config.num_sectors,
            storage_config.start_addr,
        );

        let storage_range = if start_addr == 0 {
            (flash.capacity() - storage_config.num_sectors as usize * F::ERASE_SIZE) as u32..flash.capacity() as u32
        } else {
            assert!(
                start_addr.is_multiple_of(F::ERASE_SIZE),
                "Storage's start addr MUST BE a multiplier of sector size"
            );
            start_addr as u32..(start_addr + storage_config.num_sectors as usize * F::ERASE_SIZE) as u32
        };

        let mut storage = Self {
            flash: MapStorage::new(flash, MapConfig::new(storage_range), Cache::new_uncached()),
            buffer: [0; get_buffer_size()],
            ready: false,
        };
        #[cfg(feature = "host")]
        let encoder_map = encoder_map.as_deref();

        // 自动修复可能遇到一次性 Flash 错误，重试前保留现有数据。
        for _ in 0..2 {
            match storage
                .prepare(
                    #[cfg(feature = "host")]
                    keymap,
                    #[cfg(feature = "host")]
                    encoder_map,
                    storage_config,
                    behavior_config,
                )
                .await
            {
                Ok(()) => {
                    storage.ready = true;
                    return storage;
                }
                Err(error) => print_storage_error::<F>(error),
            }
        }

        error!("Flash preparation failed, storage unavailable for this boot");
        storage
    }

    pub(crate) async fn read_behavior_config(
        &mut self,
        behavior_config: &mut config::BehaviorConfig,
    ) -> Result<(), ()> {
        let read_data = self
            .flash
            .fetch_item(&mut self.buffer, &StorageKey::BehaviorConfig)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        if let Some(StorageData::BehaviorConfig(c)) = read_data {
            behavior_config.morse.prior_idle_time = Duration::from_millis(c.prior_idle_time as u64);
            behavior_config.morse.default_profile = c.morse_default_profile;

            behavior_config.combo.timeout = Duration::from_millis(c.combo_timeout as u64);
            behavior_config.one_shot.timeout = Duration::from_millis(c.one_shot_timeout as u64);
            behavior_config.tap.tap_interval = c.tap_interval;
            behavior_config.tap.tap_capslock_interval = c.tap_capslock_interval;
        }

        Ok(())
    }

    async fn prepare(
        &mut self,
        #[cfg(feature = "host")] keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        #[cfg(feature = "host")] encoder_map: Option<&[[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        storage_config: &StorageConfig,
        behavior: &config::BehaviorConfig,
    ) -> Result<(), SSError<F::Error>> {
        if !storage_config.clear_storage
            && matches!(
                self.flash
                    .fetch_item(&mut self.buffer, &StorageKey::FormatVersion)
                    .await?,
                Some(StorageData::FormatVersion(STORAGE_FORMAT))
            )
        {
            #[cfg(feature = "host")]
            if storage_config.clear_layout {
                debug!("clear_layout=true; overwriting layout items without erase.");
                self.write_default_settings(keymap, encoder_map, behavior).await?;
            }
            return Ok(());
        }

        debug!("Clearing storage!");
        self.flash.erase_all().await?;
        self.write_default_settings(
            #[cfg(feature = "host")]
            keymap,
            #[cfg(feature = "host")]
            encoder_map,
            behavior,
        )
        .await?;

        // 写完所有默认设置后再提交标记，中途断电时下次启动会重试。
        self.store_data(StorageKey::FormatVersion, &StorageData::FormatVersion(STORAGE_FORMAT))
            .await
    }

    async fn write_default_settings(
        &mut self,
        #[cfg(feature = "host")] keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        #[cfg(feature = "host")] encoder_map: Option<&[[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        behavior: &config::BehaviorConfig,
    ) -> Result<(), SSError<F::Error>> {
        self.store_data(
            StorageKey::LayoutConfig,
            &StorageData::from(LayoutConfig {
                default_layer: 0,
                layout_option: 0,
            }),
        )
        .await?;

        self.store_data(StorageKey::BehaviorConfig, &StorageData::from(behavior))
            .await?;

        #[cfg(feature = "host")]
        for (layer, layer_data) in keymap.iter().enumerate() {
            for (row, row_data) in layer_data.iter().enumerate() {
                for (col, action) in row_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::keymap(layer as u8, row as u8, col as u8),
                        &StorageData::KeyAction(*action),
                    )
                    .await?;
                }
            }
        }

        #[cfg(feature = "host")]
        if let Some(encoder_map) = encoder_map {
            for (layer, layer_data) in encoder_map.iter().enumerate() {
                for (idx, action) in layer_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::encoder(idx as u8, layer as u8),
                        &StorageData::EncoderAction(*action),
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "vial")]
    #[doc(hidden)]
    pub async fn run_with_vial_keymap(&mut self, keymap: &crate::keymap::KeyMap<'_>) -> ! {
        self.run_storage(Some(keymap)).await
    }

    #[cfg(feature = "vial")]
    async fn write_vial_keymap(
        &mut self,
        keymap: &crate::keymap::KeyMap<'_>,
        write: VialKeymapWrite,
    ) -> Result<bool, SSError<F::Error>> {
        match write {
            VialKeymapWrite::Key {
                layer,
                row,
                col,
                action,
            } => {
                self.store_data(StorageKey::keymap(layer, row, col), &StorageData::KeyAction(action))
                    .await?;
                keymap.set_action_at(
                    crate::event::KeyboardEventPos::key_pos(col, row),
                    layer as usize,
                    action,
                );
            }
            VialKeymapWrite::Encoder {
                layer,
                idx,
                clockwise,
                action,
            } => {
                let Some(encoder) = keymap.encoder_with_direction(layer as usize, idx as usize, clockwise, action)
                else {
                    return Ok(false);
                };
                self.store_data(StorageKey::encoder(idx, layer), &StorageData::EncoderAction(encoder))
                    .await?;
                keymap.set_encoder(layer as usize, idx as usize, encoder);
            }
        }
        Ok(true)
    }

    async fn run_storage(&mut self, #[cfg(feature = "vial")] keymap: Option<&crate::keymap::KeyMap<'_>>) -> ! {
        loop {
            let info: FlashOperationMessage = FLASH_CHANNEL.receive().await;
            debug!("Flash operation: {:?}", info);

            let write_result: Result<(), SSError<F::Error>> = match info {
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadBleBondInfo(slot_num) => {
                    let resp = match self.fetch_data(StorageKey::bond_info(slot_num)).await {
                        Some(StorageData::BondInfo(info)) => Some(info),
                        _ => None,
                    };
                    BOND_INFO_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(all(feature = "_ble", feature = "split"))]
                FlashOperationMessage::ReadPeerAddress(peer_id) => {
                    let resp = match self.fetch_data(StorageKey::peer_address(peer_id)).await {
                        Some(StorageData::PeerAddress(addr)) => Some(addr),
                        _ => None,
                    };
                    PEER_ADDRESS_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadConnectionType => {
                    let resp = match self.fetch_data(StorageKey::ConnectionType).await {
                        Some(StorageData::ConnectionType(v)) => Some(v),
                        _ => None,
                    };
                    CONNECTION_TYPE_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadActiveBleProfile => {
                    let resp = match self.fetch_data(StorageKey::ActiveBleProfile).await {
                        Some(StorageData::ActiveBleProfile(v)) => Some(v),
                        _ => None,
                    };
                    ACTIVE_BLE_PROFILE_RESPONSE.signal(resp);
                    continue;
                }

                FlashOperationMessage::Reset => {
                    let result = self.flash.erase_all().await;
                    if result.is_ok() {
                        reboot_keyboard();
                    }
                    result
                }
                #[cfg(feature = "vial")]
                FlashOperationMessage::VialKeymap { id, write } => {
                    let saved = if let Some(keymap) = keymap.filter(|_| self.ready) {
                        self.write_vial_keymap(keymap, write)
                            .await
                            .map_err(print_storage_error::<F>)
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    VIAL_WRITE_RESULT.signal((id, saved));
                    continue;
                }
                _ if !self.ready => {
                    FLASH_OPERATION_FINISHED.signal(false);
                    continue;
                }
                FlashOperationMessage::LayoutOptions(layout_option) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, LayoutConfig, layout_option)
                }
                FlashOperationMessage::ResetLayout => {
                    info!("Ignoring ResetLayout at runtime (handled at startup via clear_layout).");
                    Ok(())
                }
                FlashOperationMessage::DefaultLayer(default_layer) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, LayoutConfig, default_layer)
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::MacroData(data) => {
                    self.store_data(StorageKey::MacroData, &StorageData::MacroData(data))
                        .await
                }
                #[cfg(feature = "rynk")]
                FlashOperationMessage::KeymapKey {
                    layer,
                    row,
                    col,
                    action,
                } => {
                    self.store_data(StorageKey::keymap(layer, row, col), &StorageData::KeyAction(action))
                        .await
                }
                #[cfg(feature = "rynk")]
                FlashOperationMessage::Encoder { layer, idx, action } => {
                    self.store_data(StorageKey::encoder(idx, layer), &StorageData::EncoderAction(action))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Combo { idx, config } => {
                    self.store_data(StorageKey::combo(idx), &StorageData::Combo(config))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Fork { idx, fork } => {
                    self.store_data(StorageKey::fork(idx), &StorageData::Fork(fork)).await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Morse { idx, morse } => {
                    self.store_data(StorageKey::morse(idx), &StorageData::Morse(morse))
                        .await
                }
                FlashOperationMessage::ConnectionType(ty) => {
                    self.store_data(StorageKey::ConnectionType, &StorageData::ConnectionType(ty))
                        .await
                }
                #[cfg(all(feature = "_ble", feature = "split"))]
                FlashOperationMessage::PeerAddress(peer) => {
                    self.store_data(StorageKey::peer_address(peer.peer_id), &StorageData::PeerAddress(peer))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ActiveBleProfile(profile) => {
                    self.store_data(StorageKey::ActiveBleProfile, &StorageData::ActiveBleProfile(profile))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ClearSlot(slot_num) => {
                    use trouble_host::prelude::SecurityLevel;
                    use trouble_host::{Address, BondInformation, Identity, LongTermKey};

                    info!("Clearing bond info slot_num: {}", slot_num);
                    // Remove item in `sequential-storage` is quite expensive, so just override the item with `removed = true`
                    let empty = ProfileInfo {
                        removed: true,
                        slot_num,
                        info: BondInformation::new(
                            Identity {
                                addr: Address::default(),
                                irk: None,
                            },
                            LongTermKey::from_le_bytes([0; 16]),
                            SecurityLevel::NoEncryption,
                            false,
                        ),
                        cccd_table: heapless::Vec::new(),
                    };
                    self.store_data(StorageKey::bond_info(slot_num), &StorageData::BondInfo(empty))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ProfileInfo(b) => {
                    debug!("Saving profile info: {:?}", b);
                    self.store_data(StorageKey::bond_info(b.slot_num), &StorageData::BondInfo(b))
                        .await
                }
                FlashOperationMessage::ComboTimeout(combo_timeout) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, combo_timeout)
                }
                FlashOperationMessage::OneShotTimeout(one_shot_timeout) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, one_shot_timeout)
                }
                FlashOperationMessage::TapInterval(tap_interval) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, tap_interval)
                }
                FlashOperationMessage::TapCapslockInterval(tap_capslock_interval) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, tap_capslock_interval)
                }
                FlashOperationMessage::PriorIdleTime(prior_idle_time) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, prior_idle_time)
                }
                FlashOperationMessage::MorseDefaultProfile(morse_default_profile) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, morse_default_profile)
                }
                #[cfg(feature = "rynk")]
                FlashOperationMessage::BehaviorConfig(behavior_config) => {
                    self.store_data(
                        StorageKey::BehaviorConfig,
                        &StorageData::BehaviorConfig(behavior_config),
                    )
                    .await
                }
            };

            match write_result {
                Ok(()) => FLASH_OPERATION_FINISHED.signal(true),
                Err(e) => {
                    print_storage_error::<F>(e);
                    FLASH_OPERATION_FINISHED.signal(false);
                }
            }
        }
    }
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    crate::core_traits::Runnable for Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn run(&mut self) -> ! {
        self.run_storage(
            #[cfg(feature = "vial")]
            None,
        )
        .await
    }
}

pub(crate) fn print_storage_error<F: AsyncNorFlash>(e: SSError<F::Error>) {
    match e {
        #[cfg(feature = "defmt")]
        SSError::Storage { value: e } => error!("Flash error: {:?}", defmt::Debug2Format(&e)),
        #[cfg(not(feature = "defmt"))]
        SSError::Storage { value: _e } => error!("Flash error"),
        SSError::FullStorage => error!("Storage is full"),
        SSError::Corrupted {} => error!("Storage is corrupted"),
        SSError::BufferTooBig => error!("Buffer too big"),
        SSError::BufferTooSmall(x) => error!("Buffer too small, needs {} bytes", x),
        SSError::SerializationError(e) => error!("Map value error: {}", e),
        SSError::ItemTooBig => error!("Item too big"),
        _ => error!("Unknown storage error"),
    }
}

const fn get_buffer_size() -> usize {
    #[cfg(feature = "host")]
    {
        // The buffer size needed = size_of(StorageData) = MACRO_SPACE_SIZE + 8(generally)
        // According to doc of `sequential-storage`, for some flashes it should be aligned in 32 bytes
        // To make sure the buffer works, do this alignment always
        let buffer_size = if crate::MACRO_SPACE_SIZE < 248 {
            256
        } else {
            crate::MACRO_SPACE_SIZE + 8
        };

        // Efficiently round up to the nearest multiple of 32 using bit manipulation.
        (buffer_size + 31) & !31
    }

    #[cfg(not(feature = "host"))]
    256
}

#[cfg(test)]
mod tests {
    use sequential_storage::cache::Cache;
    use sequential_storage::map::{MapConfig, MapStorage};

    use super::*;
    use crate::config::{BehaviorConfig as RuntimeBehaviorConfig, StorageConfig as RuntimeStorageConfig};
    use crate::test_support::test_block_on as block_on;

    #[cfg(feature = "vial")]
    #[test]
    fn vial_writes_finish_in_storage_even_if_host_disconnects() {
        use crate::config::PositionalConfig;
        use crate::keymap::KeymapData;
        use embassy_futures::select::{Either, select};

        block_on(async {
            let mut data =
                KeymapData::new_with_encoder([[[KeyAction::No]]], [[crate::encoder!(KeyAction::No, KeyAction::No)]]);
            let mut behavior = RuntimeBehaviorConfig::default();
            let positional = PositionalConfig::default();
            let (keymap, mut storage) = crate::initialize_keymap_and_storage(
                &mut data,
                TestFlash::<16_384, 4_096, 4>::new(),
                &RuntimeStorageConfig::default(),
                &mut behavior,
                &positional,
            )
            .await;

            let first = VialKeymapWrite::Key {
                layer: 0,
                row: 0,
                col: 0,
                action: KeyAction::Transparent,
            };
            assert!(matches!(
                select(save_vial_keymap(first), core::future::ready(())).await,
                Either::Second(())
            ));
            assert_eq!(keymap.action_at_pos(0, 0, 0), KeyAction::No);

            let change_encoder = |idx| VialKeymapWrite::Encoder {
                layer: 0,
                idx,
                clockwise: true,
                action: KeyAction::Transparent,
            };
            select(storage.run_with_vial_keymap(&keymap), async {
                assert!(!save_vial_keymap(change_encoder(1)).await);
                assert!(save_vial_keymap(change_encoder(0)).await);
                assert_eq!(keymap.action_at_pos(0, 0, 0), KeyAction::Transparent);
            })
            .await;

            assert!(matches!(
                storage.fetch_data(StorageKey::keymap(0, 0, 0)).await,
                Some(StorageData::KeyAction(KeyAction::Transparent))
            ));
            assert!(matches!(
                storage.fetch_data(StorageKey::encoder(0, 0)).await,
                Some(StorageData::EncoderAction(EncoderAction {
                    clockwise: KeyAction::Transparent,
                    counter_clockwise: KeyAction::No
                }))
            ));

            storage.flash.flash().writes_before_error = Some(0);
            select(storage.run_with_vial_keymap(&keymap), async {
                assert!(
                    !save_vial_keymap(VialKeymapWrite::Key {
                        layer: 0,
                        row: 0,
                        col: 0,
                        action: KeyAction::No
                    })
                    .await
                );
            })
            .await;
            assert_eq!(keymap.action_at_pos(0, 0, 0), KeyAction::Transparent);
        });
    }

    #[derive(Debug, Clone, Copy)]
    struct TestFlashError;

    impl embedded_storage_async::nor_flash::NorFlashError for TestFlashError {
        fn kind(&self) -> embedded_storage_async::nor_flash::NorFlashErrorKind {
            embedded_storage_async::nor_flash::NorFlashErrorKind::Other
        }
    }

    struct TestFlash<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> {
        bytes: [u8; SIZE],
        writes_before_power_cut: Option<usize>,
        writes_before_error: Option<usize>,
        fail_next_erase: bool,
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE> {
        fn new() -> Self {
            Self {
                bytes: [0xFF; SIZE],
                writes_before_power_cut: None,
                writes_before_error: None,
                fail_next_erase: false,
            }
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::ErrorType
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        type Error = TestFlashError;
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::ReadNorFlash
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const READ_SIZE: usize = 1;

        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start + bytes.len();
            bytes.copy_from_slice(&self.bytes[start..end]);
            Ok(())
        }

        fn capacity(&self) -> usize {
            SIZE
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::NorFlash
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const WRITE_SIZE: usize = WRITE_SIZE;
        const ERASE_SIZE: usize = ERASE_SIZE;

        fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            if core::mem::take(&mut self.fail_next_erase) {
                return Err(TestFlashError);
            }
            self.bytes[from as usize..to as usize].fill(0xFF);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start + bytes.len();
            for (dst, src) in self.bytes[start..end].iter_mut().zip(bytes.iter()) {
                *dst &= *src;
            }
            Ok(())
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize>
        embedded_storage_async::nor_flash::ReadNorFlash for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const READ_SIZE: usize = 1;

        async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            embedded_storage::nor_flash::ReadNorFlash::read(self, offset, bytes)
        }

        fn capacity(&self) -> usize {
            SIZE
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize>
        embedded_storage_async::nor_flash::NorFlash for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const WRITE_SIZE: usize = WRITE_SIZE;
        const ERASE_SIZE: usize = ERASE_SIZE;

        async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            embedded_storage::nor_flash::NorFlash::erase(self, from, to)
        }

        async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            if let Some(remaining) = &mut self.writes_before_error {
                *remaining = remaining.checked_sub(1).ok_or(TestFlashError)?;
            }
            if let Some(remaining) = &mut self.writes_before_power_cut {
                if *remaining == 0 {
                    return core::future::pending().await;
                }
                *remaining -= 1;
            }
            embedded_storage::nor_flash::NorFlash::write(self, offset, bytes)
        }
    }

    #[test]
    fn storage_key_round_trip() {
        let cases = [
            StorageKey::FormatVersion,
            StorageKey::LayoutConfig,
            StorageKey::BehaviorConfig,
            StorageKey::ConnectionType,
            #[cfg(feature = "host")]
            StorageKey::MacroData,
            #[cfg(feature = "host")]
            StorageKey::Keymap {
                layer: 2,
                row: 3,
                col: 4,
            },
            #[cfg(feature = "host")]
            StorageKey::Encoder { layer: 1, idx: 5 },
            #[cfg(feature = "host")]
            StorageKey::Combo(6),
            #[cfg(feature = "host")]
            StorageKey::Fork(7),
            #[cfg(feature = "host")]
            StorageKey::Morse(8),
            #[cfg(all(feature = "_ble", feature = "split"))]
            StorageKey::PeerAddress(0),
            #[cfg(feature = "_ble")]
            StorageKey::ActiveBleProfile,
            #[cfg(feature = "_ble")]
            StorageKey::BondInfo(0),
        ];

        let mut buffer = [0u8; 64];
        for key in cases {
            let size = <StorageKey as Key>::serialize_into(&key, &mut buffer).unwrap();
            let (decoded, used) = <StorageKey as Key>::deserialize_from(&buffer[..size]).unwrap();
            assert_eq!(decoded, key);
            assert_eq!(used, size);
        }
    }

    #[test]
    fn format_version_mismatch_reinitializes_storage() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; 256];

            map.store_item(
                &mut buffer,
                &StorageKey::FormatVersion,
                &StorageData::FormatVersion([STORAGE_FORMAT[0], STORAGE_FORMAT[1].wrapping_add(1)]),
            )
            .await
            .unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LayoutConfig,
                &StorageData::LayoutConfig(LayoutConfig {
                    default_layer: 7,
                    layout_option: 42,
                }),
            )
            .await
            .unwrap();

            let (flash, _) = map.destroy();
            #[cfg(feature = "host")]
            let keymap = [[[KeyAction::No; 1]; 1]; 1];
            #[cfg(feature = "host")]
            let encoder_map: Option<&mut [[EncoderAction; 0]; 1]> = None;

            let mut storage = Storage::<Flash, 1, 1, 1, 0>::new(
                flash,
                #[cfg(feature = "host")]
                &keymap,
                #[cfg(feature = "host")]
                &encoder_map,
                &RuntimeStorageConfig::default(),
                &RuntimeBehaviorConfig::default(),
            )
            .await;

            let stored_layout = storage.fetch_data(StorageKey::LayoutConfig).await.unwrap();
            let stored_config = storage.fetch_data(StorageKey::FormatVersion).await.unwrap();

            assert!(matches!(
                stored_layout,
                StorageData::LayoutConfig(LayoutConfig {
                    default_layer: 0,
                    layout_option: 0,
                })
            ));
            assert!(matches!(stored_config, StorageData::FormatVersion(STORAGE_FORMAT)));
        });
    }

    #[test]
    fn interrupted_initialization_is_retried() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;
            let mut flash = Flash::new();
            // 让布局记录先落盘，再在默认设置写完前断电。
            flash.writes_before_power_cut = Some(4);
            let config = RuntimeStorageConfig::default();
            let behavior = RuntimeBehaviorConfig::default();
            #[cfg(feature = "host")]
            let keymap = [[[KeyAction::Transparent; 1]; 1]; 1];
            #[cfg(feature = "host")]
            let mut encoders = [[crate::encoder!(KeyAction::Transparent, KeyAction::Transparent)]];
            #[cfg(feature = "host")]
            let encoder_map = Some(&mut encoders);

            let result = embassy_time::with_timeout(
                Duration::from_millis(1),
                Storage::<_, 1, 1, 1, 1>::new(
                    &mut flash,
                    #[cfg(feature = "host")]
                    &keymap,
                    #[cfg(feature = "host")]
                    &encoder_map,
                    &config,
                    &behavior,
                ),
            )
            .await;
            assert!(result.is_err());

            flash.writes_before_power_cut = None;
            let range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut interrupted =
                MapStorage::<StorageKey, _, _>::new(&mut flash, MapConfig::new(range), Cache::new_uncached());
            let mut buffer = [0; 256];
            assert!(matches!(
                interrupted.fetch_item(&mut buffer, &StorageKey::LayoutConfig).await,
                Ok(Some(StorageData::LayoutConfig(_)))
            ));
            assert!(matches!(
                interrupted
                    .fetch_item::<StorageData>(&mut buffer, &StorageKey::FormatVersion)
                    .await,
                Ok(None)
            ));

            let mut storage = Storage::<_, 1, 1, 1, 1>::new(
                flash,
                #[cfg(feature = "host")]
                &keymap,
                #[cfg(feature = "host")]
                &encoder_map,
                &config,
                &behavior,
            )
            .await;
            assert!(matches!(
                storage.fetch_data(StorageKey::FormatVersion).await,
                Some(StorageData::FormatVersion(STORAGE_FORMAT))
            ));
            assert!(matches!(
                storage.fetch_data(StorageKey::BehaviorConfig).await,
                Some(StorageData::BehaviorConfig(_))
            ));
            #[cfg(feature = "host")]
            {
                assert!(matches!(
                    storage.fetch_data(StorageKey::keymap(0, 0, 0)).await,
                    Some(StorageData::KeyAction(KeyAction::Transparent))
                ));
                assert!(matches!(
                    storage.fetch_data(StorageKey::encoder(0, 0)).await,
                    Some(StorageData::EncoderAction(EncoderAction {
                        clockwise: KeyAction::Transparent,
                        counter_clockwise: KeyAction::Transparent,
                    }))
                ));
            }
        });
    }

    #[cfg(feature = "host")]
    #[test]
    fn storage_repair_fault_retains_current_settings() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 4>;
            let mut storage = Storage::<_, 1, 1, 1, 0>::new(
                Flash::new(),
                &[[[KeyAction::No]]],
                &None,
                &RuntimeStorageConfig::default(),
                &RuntimeBehaviorConfig::default(),
            )
            .await;
            storage
                .store_data(
                    StorageKey::LayoutConfig,
                    &StorageData::LayoutConfig(LayoutConfig {
                        default_layer: 0,
                        layout_option: 42,
                    }),
                )
                .await
                .unwrap();

            let first_page = storage.flash.flash_range().start as usize;
            let second_page = first_page + 4_096;
            let (mut flash, _) = storage.flash.destroy();
            // 模拟第一页擦除中断，仅留页尾标记，正常记录留在第二页。
            flash.bytes.copy_within(first_page..second_page, second_page);
            flash.bytes[first_page..second_page].fill(0xFF);
            flash.bytes[second_page - 1] = 0;
            flash.fail_next_erase = true;

            let mut restarted = Storage::<_, 1, 1, 1, 0>::new(
                flash,
                &[[[KeyAction::No]]],
                &None,
                &RuntimeStorageConfig::default(),
                &RuntimeBehaviorConfig::default(),
            )
            .await;
            assert!(matches!(
                restarted.fetch_data(StorageKey::LayoutConfig).await,
                Some(StorageData::LayoutConfig(LayoutConfig { layout_option: 42, .. }))
            ));
        });
    }

    #[test]
    fn storage_startup_write_fault_rejects_runtime_save() {
        use crate::config::PositionalConfig;
        use crate::core_traits::Runnable;
        use crate::keymap::KeymapData;
        use embassy_futures::select::{Either, select};

        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;
            let mut flash = Flash::new();
            flash.writes_before_error = Some(4);
            let mut data = KeymapData::new([[[crate::types::action::KeyAction::No]]]);
            let mut behavior = RuntimeBehaviorConfig::default();
            let positional = PositionalConfig::default();
            let (keymap, mut storage) = crate::initialize_keymap_and_storage(
                &mut data,
                flash,
                &RuntimeStorageConfig::default(),
                &mut behavior,
                &positional,
            )
            .await;
            assert!(!storage.ready);
            assert_eq!(keymap.active_layer(), 0);
            #[cfg(feature = "vial")]
            assert_eq!(keymap.storage_ready, Some(false));
            storage.flash.flash().writes_before_error = None;
            let response = select(storage.run(), async {
                FLASH_CHANNEL.send(FlashOperationMessage::LayoutOptions(42)).await;
                FLASH_OPERATION_FINISHED.wait().await
            })
            .await;
            assert!(matches!(response, Either::Second(false)));
        });
    }

    // A stored LayoutConfig must reach the Vial GUI again after a power
    // cycle: read_keymap restores layout_option into KeymapData and
    // KeyMap::build copies it into the runtime state that
    // GetKeyboardValue(LayoutOptions) answers from (the GET wiring itself is
    // covered by host::via::tests::layout_options_set_then_get_roundtrip).
    // Deleting the restore in read_keymap (or the copy in KeyMap::build)
    // leaves the runtime value at 0 and fails this test.
    #[cfg(feature = "vial")]
    #[test]
    fn layout_option_restored_from_storage() {
        use crate::config::BehaviorConfig;
        use crate::keymap::{KeyMap, KeymapData};

        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;
            let keymap_init = [[[KeyAction::No; 1]; 1]; 1];
            let storage_config = RuntimeStorageConfig::default();
            let behavior_config = RuntimeBehaviorConfig::default();
            let boot = |flash| {
                Storage::<Flash, 1, 1, 1, 0>::new(flash, &keymap_init, &None, &storage_config, &behavior_config)
            };
            let mut storage = boot(Flash::new()).await;

            // 存储格式版本相同，重启时应保留已有设置。
            storage
                .store_data(
                    StorageKey::LayoutConfig,
                    &StorageData::LayoutConfig(LayoutConfig {
                        default_layer: 0,
                        layout_option: 42,
                    }),
                )
                .await
                .unwrap();
            #[cfg(feature = "_ble")]
            storage
                .store_data(
                    StorageKey::bond_info(2),
                    &StorageData::BondInfo(ProfileInfo {
                        slot_num: 2,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();

            let (flash, _) = storage.flash.destroy();
            let mut storage = boot(flash).await;
            #[cfg(feature = "_ble")]
            assert!(matches!(
                storage.fetch_data(StorageKey::bond_info(2)).await,
                Some(StorageData::BondInfo(ProfileInfo { slot_num: 2, .. }))
            ));

            // Boot-time restore path: storage -> KeymapData -> KeyMap::build.
            let mut data = KeymapData::new([[[KeyAction::No]]]);
            let mut behavior = BehaviorConfig::default();
            storage.read_keymap(&mut data, &mut behavior).await.unwrap();

            let positional = crate::config::PositionalConfig::<1, 1>::default();
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;

            // The freshly built keymap exposes the stored value to the via
            // GET handler.
            assert_eq!(keymap.layout_option(), 42);
        });
    }
}
