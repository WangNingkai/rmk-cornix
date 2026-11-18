//! 与硬件无关的灯效状态；调用方传入单调递增的启动毫秒数。

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Central,
    Peripheral,
}

// 动画保持平滑，静止时降低轮询频率。
pub(super) const ANIM_INTERVAL: u64 = 33;
pub(super) const IDLE_INTERVAL: u64 = 1000;
const NOTICE: u64 = 3000;
const ACTIVITY: u64 = 60_000;
const LOW_ALERT: u64 = 5000;
const LOW_QUIET: u64 = 5 * 60_000;
const FADE_STEP: u8 = 3;

const BREATH_STEPS: usize = 64;
const BREATH_PERIOD_MS: u64 = 3000;
const LOW_BLINK_PERIOD_MS: u64 = 1200;

const BATTERY_LOW: u8 = 20;
const BATTERY_FULL: u8 = 95;

const fn breath_lut() -> [u8; BREATH_STEPS] {
    let mut table = [0u8; BREATH_STEPS];
    let half = (BREATH_STEPS / 2) as u32;
    let mut i = 0usize;
    while i < BREATH_STEPS {
        let up = if (i as u32) < half {
            i as u32
        } else {
            BREATH_STEPS as u32 - i as u32
        };
        let lin = up * 1000 / half; // 线性三角波
        let gamma = lin * lin / 1000; // 平方校正亮度
        table[i] = (gamma * 255 / 1000) as u8;
        i += 1;
    }
    table
}

static BREATH: [u8; BREATH_STEPS] = breath_lut();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Grb {
    pub(super) g: u8,
    pub(super) r: u8,
    pub(super) b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LightEffect {
    Off,
    Solid(Grb),
    Breath(Grb),
    LowBattery,
}

impl LightEffect {
    fn is_animated(self) -> bool {
        matches!(self, Self::Breath(_) | Self::LowBattery)
    }
}

pub(super) const OFF: Grb = Grb { g: 0, r: 0, b: 0 };
const GREEN: Grb = Grb { g: 18, r: 0, b: 0 };
const BLUE: Grb = Grb { g: 0, r: 0, b: 18 };
const AMBER: Grb = Grb { g: 7, r: 18, b: 0 };
const CORAL: Grb = Grb { g: 8, r: 18, b: 5 };
const AMETHYST: Grb = Grb { g: 4, r: 14, b: 22 };
const EMERALD: Grb = Grb { g: 14, r: 3, b: 5 };
const SAPPHIRE: Grb = Grb { g: 3, r: 2, b: 22 };
const GARNET: Grb = Grb { g: 2, r: 20, b: 4 };
const DIAMOND: Grb = Grb {
    g: 12,
    r: 20,
    b: 16,
};

pub(super) struct IndicatorState {
    pub(super) role: Role,

    battery: Option<u8>,
    charging: bool,
    ble_profile: u8,
    ble_connected: bool,
    ble_advertising: bool,
    peer_connected: bool,
    caps_lock: bool,
    pub(super) sleeping: bool,

    ble_since: u64,
    peer_since: u64,
    charge_since: u64,
    low_alert_until: u64,
    low_next_alert: u64,

    pub(super) cur_inner: Grb,
    pub(super) cur_outer: Grb,
    tgt_inner: Grb,
    tgt_outer: Grb,
    inner_active: bool,
    outer_active: bool,
    pub(super) pending: bool,
}

impl IndicatorState {
    pub(super) fn new(role: Role, now: u64) -> Self {
        Self {
            role,
            battery: None,
            charging: false,
            ble_profile: 0,
            ble_connected: false,
            ble_advertising: false,
            peer_connected: false,
            caps_lock: false,
            sleeping: false,
            ble_since: now,
            peer_since: now,
            charge_since: now,
            low_alert_until: now,
            low_next_alert: now,
            cur_inner: OFF,
            cur_outer: OFF,
            tgt_inner: OFF,
            tgt_outer: OFF,
            inner_active: false,
            outer_active: false,
            pending: true,
        }
    }

    pub(super) fn set_charging(&mut self, charging: bool, now: u64) {
        if charging != self.charging {
            self.charging = charging;
            self.charge_since = now;
            self.pending = true;
        }
    }

    pub(super) fn set_battery_level(&mut self, level: u8, now: u64) {
        let was_full = self.battery_at_least(BATTERY_FULL);
        self.battery = Some(level);
        if self.charging && level >= BATTERY_FULL && !was_full {
            self.charge_since = now;
        }
        self.pending = true;
    }

    pub(super) fn set_peer_connected(&mut self, connected: bool, now: u64) {
        if connected != self.peer_connected {
            self.peer_since = now;
            self.peer_connected = connected;
            self.pending = true;
        }
    }

    pub(super) fn set_caps_lock(&mut self, enabled: bool) {
        if enabled != self.caps_lock {
            self.caps_lock = enabled;
            self.pending = true;
        }
    }

    pub(super) fn set_ble_state(
        &mut self,
        profile: u8,
        connected: bool,
        advertising: bool,
        now: u64,
    ) {
        let changed = connected != self.ble_connected
            || advertising != self.ble_advertising
            || profile != self.ble_profile;

        if changed {
            self.ble_since = now;
            self.pending = true;
        }

        self.ble_profile = profile;
        self.ble_connected = connected;
        self.ble_advertising = advertising;
    }

    pub(super) fn set_sleeping(&mut self, sleeping: bool) {
        if self.sleeping != sleeping {
            self.sleeping = sleeping;
            self.pending = true;
        }
    }

    fn profile_color(&self) -> Grb {
        match self.ble_profile {
            0 => AMETHYST,
            1 => EMERALD,
            2 => SAPPHIRE,
            3 => GARNET,
            _ => DIAMOND,
        }
    }

    fn breath_level(&self, now: u64) -> u8 {
        let ms = now % BREATH_PERIOD_MS;
        let idx = (ms as usize * BREATH_STEPS / BREATH_PERIOD_MS as usize) % BREATH_STEPS;
        BREATH[idx]
    }

    fn breath_color(&self, color: Grb, now: u64) -> Grb {
        let level = self.breath_level(now) as u16;
        Grb {
            g: (color.g as u16 * level / 255) as u8,
            r: (color.r as u16 * level / 255) as u8,
            b: (color.b as u16 * level / 255) as u8,
        }
    }

    fn low_blink_on(&self, now: u64) -> bool {
        let ms = now % LOW_BLINK_PERIOD_MS;
        ms < 200 || (400..600).contains(&ms)
    }

    fn effect_color(&self, effect: LightEffect, now: u64) -> Grb {
        match effect {
            LightEffect::Off => OFF,
            LightEffect::Solid(color) => color,
            LightEffect::Breath(color) => self.breath_color(color, now),
            LightEffect::LowBattery => {
                if self.low_blink_on(now) {
                    CORAL
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

    fn low_blinking(&self, now: u64) -> bool {
        self.battery_at_most(BATTERY_LOW) && now < self.low_alert_until
    }

    fn inner_effect(&self, now: u64) -> LightEffect {
        if self.charging {
            if self.battery_at_least(BATTERY_FULL) {
                return if (now - self.charge_since) < NOTICE {
                    LightEffect::Solid(GREEN)
                } else {
                    LightEffect::Off
                };
            }
            return LightEffect::Breath(GREEN);
        }

        if self.low_blinking(now) {
            return LightEffect::LowBattery;
        }

        if self.role == Role::Central {
            if !self.peer_connected {
                return if (now - self.peer_since) < ACTIVITY {
                    LightEffect::Breath(BLUE)
                } else {
                    LightEffect::Off
                };
            }
            if (now - self.peer_since) < NOTICE {
                return LightEffect::Solid(BLUE);
            }
        }

        LightEffect::Off
    }

    fn outer_effect(&self, now: u64) -> LightEffect {
        match self.role {
            Role::Central => {
                if self.ble_connected && (now - self.ble_since) < NOTICE {
                    LightEffect::Solid(self.profile_color())
                } else if self.ble_advertising && (now - self.ble_since) < ACTIVITY {
                    LightEffect::Breath(self.profile_color())
                } else if self.caps_lock {
                    LightEffect::Solid(AMBER)
                } else {
                    LightEffect::Off
                }
            }
            Role::Peripheral => {
                if self.peer_connected {
                    if (now - self.peer_since) < NOTICE {
                        LightEffect::Solid(BLUE)
                    } else {
                        LightEffect::Off
                    }
                } else if (now - self.peer_since) < ACTIVITY {
                    LightEffect::Breath(BLUE)
                } else {
                    LightEffect::Off
                }
            }
        }
    }

    // 电量恢复时重置计划，保证再次低电量能够立即提醒。
    fn refresh_low_battery(&mut self, now: u64) {
        if !self.battery_at_most(BATTERY_LOW) {
            self.low_alert_until = now;
            self.low_next_alert = now;
            return;
        }
        if now >= self.low_next_alert {
            self.low_alert_until = now + LOW_ALERT;
            self.low_next_alert = now + LOW_ALERT + LOW_QUIET;
        }
    }

    // 呼吸谷底和闪烁暗相仍需保持动画刷新。
    pub(super) fn is_animating(&self) -> bool {
        self.inner_active
            || self.outer_active
            || self.cur_inner != self.tgt_inner
            || self.cur_outer != self.tgt_outer
    }

    fn approach_channel(cur: u8, target: u8, step: u8) -> u8 {
        if cur < target {
            cur.saturating_add(step).min(target)
        } else if cur > target {
            cur.saturating_sub(step).max(target)
        } else {
            cur
        }
    }

    fn approach(cur: Grb, target: Grb, step: u8) -> Grb {
        Grb {
            g: Self::approach_channel(cur.g, target.g, step),
            r: Self::approach_channel(cur.r, target.r, step),
            b: Self::approach_channel(cur.b, target.b, step),
        }
    }

    pub(super) fn update(&mut self, now: u64) {
        if self.sleeping {
            self.inner_active = false;
            self.outer_active = false;
            self.tgt_inner = OFF;
            self.tgt_outer = OFF;
        } else {
            self.refresh_low_battery(now);
            let inner = self.inner_effect(now);
            let outer = self.outer_effect(now);
            self.inner_active = inner.is_animated();
            self.outer_active = outer.is_animated();
            self.tgt_inner = self.effect_color(inner, now);
            self.tgt_outer = self.effect_color(outer, now);
        }

        self.cur_inner = Self::approach(self.cur_inner, self.tgt_inner, FADE_STEP);
        self.cur_outer = Self::approach(self.cur_outer, self.tgt_outer, FADE_STEP);
        self.pending = false;
    }

    pub(super) fn is_suspended(&self) -> bool {
        self.sleeping && !self.pending && !self.is_animating()
    }
}

#[cfg(test)]
#[path = "../../tests/ws2812/state.rs"]
mod tests;
