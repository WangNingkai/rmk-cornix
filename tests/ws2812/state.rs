use super::*;

fn advance_frames(state: &mut IndicatorState, now: &mut u64, frames: usize) {
    for _ in 0..frames {
        *now += ANIM_INTERVAL;
        state.update(*now);
    }
}

#[test]
fn low_battery_rearms_immediately_after_recovery() {
    let mut state = IndicatorState::new(Role::Peripheral, 0);
    state.set_battery_level(BATTERY_LOW, 0);
    state.update(0);
    assert_eq!(state.inner_effect(0), LightEffect::LowBattery);

    state.update(LOW_ALERT);
    assert_eq!(state.inner_effect(LOW_ALERT), LightEffect::Off);
    let recovered_at = LOW_ALERT + IDLE_INTERVAL;
    state.set_battery_level(BATTERY_LOW + 1, recovered_at);
    state.update(recovered_at);

    let low_again_at = recovered_at + ANIM_INTERVAL;
    state.set_battery_level(BATTERY_LOW, low_again_at);
    state.update(low_again_at);
    assert_eq!(state.inner_effect(low_again_at), LightEffect::LowBattery);
    assert_eq!(state.low_alert_until, low_again_at + LOW_ALERT);

    state.update(low_again_at + LOW_ALERT);
    assert_eq!(
        state.inner_effect(low_again_at + LOW_ALERT),
        LightEffect::Off
    );
    let next_alert = low_again_at + LOW_ALERT + LOW_QUIET;
    state.update(next_alert - 1);
    assert_eq!(state.inner_effect(next_alert - 1), LightEffect::Off);
    state.update(next_alert);
    assert_eq!(state.inner_effect(next_alert), LightEffect::LowBattery);
}

#[test]
fn sleep_fades_both_halves_to_black_before_suspending() {
    for role in [Role::Central, Role::Peripheral] {
        let mut state = IndicatorState::new(role, 0);
        state.set_peer_connected(true, 0);
        state.set_ble_state(0, true, false, 0);
        let mut now = 0;
        advance_frames(&mut state, &mut now, 8);
        assert_eq!(
            state.cur_outer,
            if role == Role::Central {
                AMETHYST
            } else {
                BLUE
            }
        );

        state.set_sleeping(true);
        assert!(!state.is_suspended());
        advance_frames(&mut state, &mut now, 1);
        assert_ne!(state.cur_outer, OFF);
        assert!(state.is_animating());
        assert!(!state.is_suspended());

        advance_frames(&mut state, &mut now, 8);
        assert_eq!((state.cur_inner, state.cur_outer), (OFF, OFF));
        assert!(!state.is_animating());
        assert!(state.is_suspended());

        state.set_sleeping(false);
        assert!(!state.is_suspended());
        advance_frames(&mut state, &mut now, 1);
        assert_ne!(state.cur_outer, OFF);
    }
}

#[test]
fn connection_and_advertising_notices_restore_caps_lock() {
    let mut state = IndicatorState::new(Role::Central, 0);
    state.set_caps_lock(true);
    state.set_ble_state(0, true, false, 0);
    assert_eq!(state.outer_effect(NOTICE - 1), LightEffect::Solid(AMETHYST));
    assert_eq!(state.outer_effect(NOTICE), LightEffect::Solid(AMBER));

    state.set_ble_state(1, true, false, NOTICE);
    assert_eq!(state.outer_effect(NOTICE), LightEffect::Solid(EMERALD));
    assert_eq!(state.outer_effect(NOTICE * 2), LightEffect::Solid(AMBER));

    state.set_ble_state(1, false, true, NOTICE * 2);
    let expiry = NOTICE * 2 + ACTIVITY;
    assert_eq!(state.outer_effect(expiry - 1), LightEffect::Breath(EMERALD));
    assert_eq!(state.outer_effect(expiry), LightEffect::Solid(AMBER));
    let mut now = expiry;
    advance_frames(&mut state, &mut now, 8);
    assert_eq!(state.cur_outer, AMBER);
    assert!(!state.is_animating());
    assert!(!state.is_suspended());
}

#[test]
fn charging_overrides_low_battery_and_full_notice_expires() {
    let mut state = IndicatorState::new(Role::Central, 0);
    state.set_battery_level(BATTERY_LOW, 0);
    state.set_charging(true, 0);
    state.update(0);
    assert_eq!(state.inner_effect(0), LightEffect::Breath(GREEN));

    state.set_battery_level(BATTERY_FULL, 1000);
    assert_eq!(state.inner_effect(1000), LightEffect::Solid(GREEN));
    assert_eq!(
        state.inner_effect(1000 + NOTICE - 1),
        LightEffect::Solid(GREEN)
    );
    assert_eq!(state.inner_effect(1000 + NOTICE), LightEffect::Off);

    // 后续满电采样不能反复延长提示。
    state.set_battery_level(100, 1000 + NOTICE);
    assert_eq!(state.inner_effect(1000 + NOTICE), LightEffect::Off);
    state.set_charging(false, 1000 + NOTICE);
    assert_eq!(state.inner_effect(1000 + NOTICE), LightEffect::Breath(BLUE));
}

#[test]
fn animation_keeps_running_during_dark_phases() {
    let mut state = IndicatorState::new(Role::Peripheral, 0);
    state.update(0);
    assert_eq!(state.cur_outer, OFF);
    assert!(state.is_animating());

    state.set_peer_connected(true, 0);
    state.set_battery_level(BATTERY_LOW, ACTIVITY);
    state.update(ACTIVITY + 300);
    assert_eq!(state.cur_inner, OFF);
    assert!(state.is_animating());
}
