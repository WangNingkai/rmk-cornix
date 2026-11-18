use embassy_nrf::gpio::Output;
use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Instant, Timer};
use rmk::embassy_futures::select::{select, Either};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ChargingStateEvent, ConnectionStatusChangeEvent,
    EventSubscriber, LedIndicatorEvent, PeripheralConnectedEvent, SleepStateEvent,
};
use rmk::processor::{PollingProcessor, Processor};
use rmk::types::battery::BatteryStatus;
use rmk::types::ble::BleState;

#[path = "ws2812/state.rs"]
mod state;

pub use state::Role;
use state::{Grb, IndicatorState, ANIM_INTERVAL, IDLE_INTERVAL, OFF};

pub const PWM_TOP: u16 = 20;
const W0: u16 = 0x8000 | 6;
const W1: u16 = 0x8000 | 13;
const WRESET: u16 = 0x8000;
const SEQ_BITS: usize = 2 * 3 * 8;
const SEQ_RESET: usize = 40;
const SEQ_LEN: usize = SEQ_BITS + SEQ_RESET;

#[rmk::macros::processor(subscribe = [
    BatteryStatusEvent, ChargingStateEvent, PeripheralConnectedEvent, CentralConnectedEvent,
    ConnectionStatusChangeEvent, LedIndicatorEvent, SleepStateEvent,
])]
pub struct Ws2812Indicator {
    pwm: SequencePwm<'static>,
    ext_power: Output<'static>,
    state: IndicatorState,
    rail_on: bool,
    last: Option<(Grb, Grb)>,
}

impl Ws2812Indicator {
    pub fn new(pwm: SequencePwm<'static>, mut ext_power: Output<'static>, role: Role) -> Self {
        ext_power.set_low();
        Self {
            pwm,
            ext_power,
            state: IndicatorState::new(role, Instant::now().as_millis()),
            rail_on: false,
            last: None,
        }
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

        // Only cut the rail once the fade-out has fully settled to off, so the
        // last visible frame is black rather than an abrupt power cut.
        if !any_on {
            self.ext_power.set_low();
            self.rail_on = false;
        }
    }
}

impl Ws2812Indicator {
    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        if let BatteryStatus::Available {
            level: Some(level), ..
        } = event.0
        {
            self.state
                .set_battery_level(level, Instant::now().as_millis());
        }
    }

    async fn on_charging_state_event(&mut self, event: ChargingStateEvent) {
        self.state
            .set_charging(event.charging, Instant::now().as_millis());
    }

    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        if self.state.role == Role::Central {
            self.state
                .set_peer_connected(event.connected, Instant::now().as_millis());
        }
    }

    async fn on_central_connected_event(&mut self, event: CentralConnectedEvent) {
        if self.state.role == Role::Peripheral {
            self.state
                .set_peer_connected(event.connected, Instant::now().as_millis());
        }
    }

    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        if self.state.role == Role::Central {
            self.state.set_ble_state(
                event.0.ble.profile,
                matches!(event.0.ble.state, BleState::Connected),
                matches!(event.0.ble.state, BleState::Advertising),
                Instant::now().as_millis(),
            );
        }
    }

    async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
        if self.state.role == Role::Central {
            self.state.set_caps_lock(event.0.caps_lock());
        }
    }

    async fn on_sleep_state_event(&mut self, event: SleepStateEvent) {
        self.state.set_sleeping(event.0);
    }
}

impl PollingProcessor for Ws2812Indicator {
    // 实际间隔由下方自适应循环决定。
    fn interval(&self) -> Duration {
        Duration::from_millis(ANIM_INTERVAL)
    }

    async fn update(&mut self) {
        let now = Instant::now().as_millis();
        if !self.state.sleeping {
            let usb_present = embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect();
            self.state.set_charging(usb_present, now);
        }
        self.state.update(now);
        self.render(self.state.cur_inner, self.state.cur_outer)
            .await;
    }

    /// 动画按 33 ms 刷新，静止时降至 1 秒；绝对截止时间防止连续事件推迟刷新。
    async fn polling_loop(&mut self) -> ! {
        let mut sub = Self::subscriber();
        let mut deadline = Instant::now();
        loop {
            // When sleeping and fully settled (LEDs off), suspend periodic
            // polling entirely — just wait for controller events.  This keeps
            // the CPU in WFE between BLE connection events and eliminates the
            // 1-second idle tick that was waking it for no reason.
            if self.state.is_suspended() {
                loop {
                    let event = sub.next_event().await;
                    self.process(event).await;
                    if self.state.pending {
                        break;
                    }
                }
                // A state-changing event arrived; resume the normal loop so
                // `update()` can react (e.g. fade-in on wake, show charging).
                deadline = Instant::now();
                continue;
            }

            let now = Instant::now();
            if now >= deadline {
                self.update().await;
                let interval = if self.state.is_animating() {
                    ANIM_INTERVAL
                } else {
                    IDLE_INTERVAL
                };
                deadline = Instant::now() + Duration::from_millis(interval);
                continue;
            }

            match select(Timer::after(deadline - now), sub.next_event()).await {
                Either::First(_) => {
                    self.update().await;
                    let interval = if self.state.is_animating() {
                        ANIM_INTERVAL
                    } else {
                        IDLE_INTERVAL
                    };
                    deadline = Instant::now() + Duration::from_millis(interval);
                }
                Either::Second(event) => {
                    self.process(event).await;
                    // Reflect a state-changing event promptly by pulling the
                    // deadline in; never push it out (ignored events keep it).
                    if self.state.pending {
                        let soon = Instant::now() + Duration::from_millis(ANIM_INTERVAL);
                        if soon < deadline {
                            deadline = soon;
                        }
                    }
                }
            }
        }
    }
}
