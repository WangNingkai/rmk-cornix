use embassy_nrf::gpio::Output;
use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_nrf::saadc::Saadc;
use embassy_time::{Duration, Timer};
use rmk::ble::BleState;
use rmk::channel::{ControllerSub, CONTROLLER_CHANNEL};
use rmk::controller::{Controller, PollingController};
use rmk::event::ControllerEvent;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Central,
    Peripheral,
}

const POLL_INTERVAL_MS: u32 = 100;
const BREATH_FRAMES: u32 = 30;
const LOW_BLINK_PERIOD: u32 = 12;
const NOTICE_FRAMES: u32 = 3_000 / POLL_INTERVAL_MS;
const ACTIVITY_FRAMES: u32 = 60_000 / POLL_INTERVAL_MS;
const LOW_ALERT_FRAMES: u32 = 5_000 / POLL_INTERVAL_MS;
const LOW_REMINDER_FRAMES: u32 = 5 * 60_000 / POLL_INTERVAL_MS;
const BATTERY_SAMPLE_FRAMES: u32 = 30_000 / POLL_INTERVAL_MS;
const LEVEL: u8 = 0x10;
const BREATH_PEAK: u8 = 0x20;
const BATTERY_LOW: u8 = 20;
const BATTERY_FULL: u8 = 95;
const ADC_DIVIDER_MEASURED: i32 = 2000;
const ADC_DIVIDER_TOTAL: i32 = 2806;

pub const PWM_TOP: u16 = 20;
const W0: u16 = 0x8000 | 6;
const W1: u16 = 0x8000 | 13;
const WRESET: u16 = 0x8000;
const SEQ_BITS: usize = 2 * 3 * 8;
const SEQ_RESET: usize = 40;
const SEQ_LEN: usize = SEQ_BITS + SEQ_RESET;

const fn breath_table() -> [u8; BREATH_FRAMES as usize] {
    let mut table = [0u8; BREATH_FRAMES as usize];
    let half = BREATH_FRAMES / 2;
    let mut i = 0u32;
    while i < BREATH_FRAMES {
        let up = if i <= half { i } else { BREATH_FRAMES - i };
        table[i as usize] = ((up * BREATH_PEAK as u32) / half) as u8;
        i += 1;
    }
    table
}

static BREATH: [u8; BREATH_FRAMES as usize] = breath_table();

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Grb {
    g: u8,
    r: u8,
    b: u8,
}

#[derive(Clone, Copy)]
enum LightEffect {
    Off,
    Solid(Grb),
    Breath(Grb),
    LowBattery,
}

const OFF: Grb = Grb { g: 0, r: 0, b: 0 };
const RED: Grb = Grb {
    g: 0,
    r: LEVEL,
    b: 0,
};
const GREEN: Grb = Grb {
    g: LEVEL,
    r: 0,
    b: 0,
};
const BLUE: Grb = Grb {
    g: 0,
    r: 0,
    b: LEVEL,
};

pub struct Ws2812Indicator {
    pwm: SequencePwm<'static>,
    ext_power: Output<'static>,
    battery_adc: Option<Saadc<'static, 1>>,
    role: Role,
    sub: ControllerSub,

    battery: Option<u8>,
    charging: bool,
    ble_profile: u8,
    ble_connected: bool,
    ble_advertising: bool,
    peer_connected: bool,
    sleeping: bool,

    tick: u32,
    ble_frame: u32,
    peer_frame: u32,
    charge_frame: u32,
    low_alert_frame: u32,
    low_reminder_frame: u32,
    battery_sample_frame: u32,
    rail_on: bool,
    last: Option<(Grb, Grb)>,
}

impl Ws2812Indicator {
    pub fn new(
        pwm: SequencePwm<'static>,
        mut ext_power: Output<'static>,
        battery_adc: Option<Saadc<'static, 1>>,
        role: Role,
    ) -> Self {
        ext_power.set_low();
        Self {
            pwm,
            ext_power,
            battery_adc,
            role,
            sub: match CONTROLLER_CHANNEL.subscriber() {
                Ok(sub) => sub,
                Err(_) => panic!("controller subscriber unavailable"),
            },
            battery: None,
            charging: false,
            ble_profile: 0,
            ble_connected: false,
            ble_advertising: false,
            peer_connected: false,
            sleeping: false,
            tick: 0,
            ble_frame: 0,
            peer_frame: 0,
            charge_frame: 0,
            low_alert_frame: LOW_ALERT_FRAMES,
            low_reminder_frame: 0,
            battery_sample_frame: 0,
            rail_on: false,
            last: None,
        }
    }

    fn set_charging(&mut self, charging: bool) {
        if charging != self.charging {
            self.charging = charging;
            self.charge_frame = 0;
        }
    }

    fn set_battery_level(&mut self, level: u8) {
        let was_low = self.battery_at_most(BATTERY_LOW);
        let was_full = self.battery_at_least(BATTERY_FULL);

        self.battery = Some(level);

        if level <= BATTERY_LOW && !was_low {
            self.reset_low_alert();
        } else if level > BATTERY_LOW {
            self.low_alert_frame = LOW_ALERT_FRAMES;
            self.low_reminder_frame = 0;
        }

        if self.charging && level >= BATTERY_FULL && !was_full {
            self.charge_frame = 0;
        }
    }

    fn set_peer_connected(&mut self, connected: bool) {
        if connected != self.peer_connected {
            self.peer_frame = 0;
        }
        self.peer_connected = connected;
    }

    fn set_ble_state(&mut self, profile: u8, state: BleState) {
        let connected = matches!(state, BleState::Connected);
        let advertising = matches!(state, BleState::Advertising);
        let changed = connected != self.ble_connected
            || advertising != self.ble_advertising
            || profile != self.ble_profile;

        if changed {
            self.ble_frame = 0;
        }

        self.ble_profile = profile;
        self.ble_connected = connected;
        self.ble_advertising = advertising;
    }

    fn set_sleeping(&mut self, sleeping: bool) {
        if self.sleeping != sleeping && !sleeping {
            self.ble_frame = 0;
            self.peer_frame = 0;
        }
        self.sleeping = sleeping;
    }

    fn reset_low_alert(&mut self) {
        self.low_alert_frame = 0;
        self.low_reminder_frame = 0;
    }

    fn profile_color(&self) -> Grb {
        match self.ble_profile {
            0 => RED,
            1 => GREEN,
            2 => BLUE,
            _ => BLUE,
        }
    }

    fn low_blink_on(&self) -> bool {
        let phase = self.tick % LOW_BLINK_PERIOD;
        phase < 2 || (4..6).contains(&phase)
    }

    fn breath_color(&self, color: Grb) -> Grb {
        let level = BREATH[(self.tick % BREATH_FRAMES) as usize];
        Grb {
            g: if color.g > 0 { level } else { 0 },
            r: if color.r > 0 { level } else { 0 },
            b: if color.b > 0 { level } else { 0 },
        }
    }

    fn effect_color(&self, effect: LightEffect) -> Grb {
        match effect {
            LightEffect::Off => OFF,
            LightEffect::Solid(color) => color,
            LightEffect::Breath(color) => self.breath_color(color),
            LightEffect::LowBattery => {
                if self.low_blink_on() {
                    RED
                } else {
                    OFF
                }
            }
        }
    }

    fn battery_at_least(&self, threshold: u8) -> bool {
        matches!(self.battery, Some(level) if level >= threshold)
    }

    fn battery_at_most(&self, threshold: u8) -> bool {
        matches!(self.battery, Some(level) if level <= threshold)
    }

    fn low_battery_effect(&self) -> Option<LightEffect> {
        if self.battery_at_most(BATTERY_LOW) && self.low_alert_frame < LOW_ALERT_FRAMES {
            Some(LightEffect::LowBattery)
        } else {
            None
        }
    }

    fn inner_effect(&self) -> LightEffect {
        if self.charging {
            if self.battery_at_least(BATTERY_FULL) {
                return if self.charge_frame < NOTICE_FRAMES {
                    LightEffect::Solid(GREEN)
                } else {
                    LightEffect::Off
                };
            }
            return LightEffect::Breath(GREEN);
        }

        if let Some(effect) = self.low_battery_effect() {
            return effect;
        }

        if self.role == Role::Central {
            if !self.peer_connected {
                return if self.peer_frame < ACTIVITY_FRAMES {
                    LightEffect::Breath(BLUE)
                } else {
                    LightEffect::Off
                };
            }
            if self.peer_frame < NOTICE_FRAMES {
                return LightEffect::Solid(BLUE);
            }
        }

        LightEffect::Off
    }

    fn outer_effect(&self) -> LightEffect {
        match self.role {
            Role::Central => {
                if self.ble_connected {
                    if self.ble_frame < NOTICE_FRAMES {
                        LightEffect::Solid(self.profile_color())
                    } else {
                        LightEffect::Off
                    }
                } else if self.ble_advertising && self.ble_frame < ACTIVITY_FRAMES {
                    LightEffect::Breath(self.profile_color())
                } else {
                    LightEffect::Off
                }
            }
            Role::Peripheral => {
                if self.peer_connected {
                    if self.peer_frame < NOTICE_FRAMES {
                        LightEffect::Solid(BLUE)
                    } else {
                        LightEffect::Off
                    }
                } else if self.peer_frame < ACTIVITY_FRAMES {
                    LightEffect::Breath(BLUE)
                } else {
                    LightEffect::Off
                }
            }
        }
    }

    fn battery_percent_from_adc(val: i16) -> u8 {
        let val = val as i32;
        let full = 4755 * ADC_DIVIDER_MEASURED / ADC_DIVIDER_TOTAL;
        let empty = 4055 * ADC_DIVIDER_MEASURED / ADC_DIVIDER_TOTAL;

        if val > full {
            100
        } else if val < empty {
            0
        } else {
            ((val * ADC_DIVIDER_TOTAL / ADC_DIVIDER_MEASURED - 4055) / 7) as u8
        }
    }

    async fn sample_battery_if_due(&mut self) {
        if self.battery_sample_frame != 0 {
            return;
        }

        let Some(battery_adc) = self.battery_adc.as_mut() else {
            return;
        };

        let mut buf = [0i16; 1];
        battery_adc.sample(&mut buf).await;
        self.set_battery_level(Self::battery_percent_from_adc(buf[0]));
    }

    fn update_low_battery_timers(&mut self) {
        if !self.battery_at_most(BATTERY_LOW) {
            self.low_alert_frame = LOW_ALERT_FRAMES;
            self.low_reminder_frame = 0;
            return;
        }

        if self.low_alert_frame < LOW_ALERT_FRAMES {
            self.low_alert_frame += 1;
        } else {
            self.low_reminder_frame = self.low_reminder_frame.saturating_add(1);
            if self.low_reminder_frame >= LOW_REMINDER_FRAMES {
                self.reset_low_alert();
            }
        }
    }

    fn advance_frames(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.ble_frame = self.ble_frame.saturating_add(1);
        self.peer_frame = self.peer_frame.saturating_add(1);
        self.charge_frame = self.charge_frame.saturating_add(1);
        self.battery_sample_frame = (self.battery_sample_frame + 1) % BATTERY_SAMPLE_FRAMES;
        self.update_low_battery_timers();
    }

    fn encode(buf: &mut [u16; SEQ_LEN], inner: Grb, outer: Grb) {
        let bytes = [inner.g, inner.r, inner.b, outer.g, outer.r, outer.b];
        let mut k = 0;
        for byte in bytes {
            let mut value = byte;
            for _ in 0..8 {
                buf[k] = if value & 0x80 != 0 { W1 } else { W0 };
                k += 1;
                value <<= 1;
            }
        }
        while k < SEQ_LEN {
            buf[k] = WRESET;
            k += 1;
        }
    }

    async fn render(&mut self, inner: Grb, outer: Grb) {
        if self.last == Some((inner, outer)) {
            return;
        }
        self.last = Some((inner, outer));

        let any_on = inner != OFF || outer != OFF;
        if any_on && !self.rail_on {
            self.ext_power.set_high();
            Timer::after(Duration::from_millis(5)).await;
            self.rail_on = true;
        }

        let mut buf = [WRESET; SEQ_LEN];
        Self::encode(&mut buf, inner, outer);
        {
            let seq = SingleSequencer::new(&mut self.pwm, &buf, SequenceConfig::default());
            if seq.start(SingleSequenceMode::Times(1)).is_ok() {
                Timer::after(Duration::from_millis(1)).await;
            }
        }

        if !any_on {
            self.ext_power.set_low();
            self.rail_on = false;
        }
    }
}

impl Controller for Ws2812Indicator {
    type Event = ControllerEvent;

    async fn process_event(&mut self, event: Self::Event) {
        match event {
            ControllerEvent::Battery(level) => self.set_battery_level(level),
            ControllerEvent::ChargingState(charging) => self.set_charging(charging),
            ControllerEvent::SplitPeripheral(_, connected) if self.role == Role::Central => {
                self.set_peer_connected(connected);
            }
            ControllerEvent::SplitCentral(connected) if self.role == Role::Peripheral => {
                self.set_peer_connected(connected);
            }
            ControllerEvent::BleState(profile, state) if self.role == Role::Central => {
                self.set_ble_state(profile, state);
            }
            ControllerEvent::BleProfile(profile) if self.role == Role::Central => {
                if profile != self.ble_profile {
                    self.ble_profile = profile;
                    self.ble_frame = 0;
                }
            }
            ControllerEvent::Sleep(sleeping) => self.set_sleeping(sleeping),
            _ => {}
        }
    }

    async fn next_message(&mut self) -> Self::Event {
        self.sub.next_message_pure().await
    }
}

impl PollingController for Ws2812Indicator {
    const INTERVAL: Duration = Duration::from_millis(POLL_INTERVAL_MS as u64);

    async fn update(&mut self) {
        if self.sleeping {
            self.render(OFF, OFF).await;
            return;
        }

        let usb_present = embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect();
        self.set_charging(usb_present);
        self.sample_battery_if_due().await;

        let inner = self.effect_color(self.inner_effect());
        let outer = self.effect_color(self.outer_effect());
        self.render(inner, outer).await;
        self.advance_frames();
    }
}
