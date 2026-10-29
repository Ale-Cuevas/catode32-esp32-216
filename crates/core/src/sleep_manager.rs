//! Manages device sleep modes for power saving.
//!
//! Currently implements only the "basic" mode:
//!
//!   - After `SLEEP_TIMEOUT` of no button activity, power off the display.
//!   - Continue ticking the game at `SLEEP_FPS` so needs/time advance while
//!     invisible.
//!   - Any button press wakes the device; the wake press is consumed so it
//!     does not register as a game action.
//!   - On wake, set `ctx.pending_wake_greeting` so the pet greets the
//!     returning player on its next behavior pick, and signal the active
//!     scene's behavior manager to wind down quickly so the greeting fires
//!     promptly rather than at the next natural completion.
//!   - On battery only (`platform::power::status`), a critically low
//!     battery ends basic sleep in deep sleep instead (`deep_sleep_due`).
//!     Not sooner: without an RTC the pet's clock stops in deep sleep,
//!     while basic sleep keeps it running. On USB it never happens.
//!
//! TODO(deep_sleep): implement a "deep" mode (true MCU light/deep sleep with
//! GPIO IRQ wake) once esp-hal's sleep APIs are wired up.
//!
//! TODO(gpio_irq_wake): wake detection currently polls every button in
//! the slow loop at `SLEEP_FPS` (option A from the design discussion).
//! A falling-edge IRQ on each button would let the CPU idle until a
//! press, which actually saves CPU power between ticks. Move to esp-hal
//! GPIO IRQ + light sleep to match that intent. Until then the
//! screen-off saving (~1-2 mA) is the only real power benefit of basic
//! mode.

use crate::platform::power::{self, PowerStatus};
use crate::platform::time::{Duration, Instant};
use crate::platform::watchdog;

use crate::{
    context::GameContext,
    input::{Button, Buttons},
    render::Renderer,
    scene::SceneManager,
    time_system::TimeSystem,
};

/// Spin until none of the given buttons (or `Button::ALL` if `mask` is empty)
/// has been pressed for `stable` continuously. Any press resets the timer.
pub fn wait_buttons_stable_released(buttons: &Buttons, stable: Duration) {
    wait_buttons_stable_released_mask(buttons, &Button::ALL, stable);
}

pub fn wait_buttons_stable_released_mask(
    buttons: &Buttons,
    watch: &[Button],
    stable: Duration,
) {
    let mut released_since = Instant::now();
    loop {
        // A held button can keep this spinning indefinitely.
        watchdog::feed();
        let any_pressed = watch.iter().any(|&b| buttons.is_pressed(b));
        if any_pressed {
            released_since = Instant::now();
        } else if released_since.elapsed() >= stable {
            return;
        }
    }
}

/// Seconds of inactivity before sleeping.
pub const SLEEP_TIMEOUT: Duration = Duration::from_secs(900);

/// Game update rate while in basic sleep.
pub const SLEEP_FPS: u64 = 2;
pub const SLEEP_FRAME_TIME_MS: u64 = 1000 / SLEEP_FPS;

/// Battery level at or below which basic sleep turns into deep sleep.
pub const DEEP_SLEEP_BATTERY_PCT: u8 = 5;

/// How basic sleep ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepExit {
    /// A button woke it; the display is back on.
    Woken,
    /// The battery ran low (`deep_sleep_due`); the display is still off and
    /// the caller should save and deep sleep.
    DeepSleep,
}

/// Deep sleep only on a known, discharging, critically low battery. No
/// reading, no battery or USB power all keep the device in basic sleep.
pub fn deep_sleep_due(status: Option<PowerStatus>) -> bool {
    matches!(
        status,
        Some(PowerStatus { battery_pct: Some(pct), on_usb: false, .. }) if pct <= DEEP_SLEEP_BATTERY_PCT
    )
}

pub struct SleepManager {
    sleeping: bool,
    last_activity: Instant,
}

impl SleepManager {
    pub fn new() -> Self {
        Self {
            sleeping: false,
            last_activity: Instant::now(),
        }
    }

    #[allow(dead_code)]
    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    /// Reset the inactivity timer. Call from the main loop on any button press.
    pub fn notify_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// True when the inactivity timeout has elapsed and sleep is appropriate.
    ///
    /// Desktop builds never auto-sleep. The basic-sleep path runs an
    /// internal `while !buttons.any_pressed()` loop that doesn't yield
    /// back to the desktop binary's SDL event pump, so the simulator
    /// would freeze instead of sleeping.
    pub fn should_sleep(&self) -> bool {
        #[cfg(feature = "desktop")]
        {
            return false;
        }
        #[cfg(not(feature = "desktop"))]
        {
            if self.sleeping {
                return false;
            }
            self.last_activity.elapsed() >= SLEEP_TIMEOUT
        }
    }

    /// Block in basic sleep until any button is pressed, or until the
    /// battery says to deep sleep (`SleepExit::DeepSleep`).
    ///
    /// The display is powered off for the duration; the game keeps ticking
    /// at `SLEEP_FPS` so needs and time still advance while the screen is
    /// dark. On wake we restore the display, consume the wake press, and
    /// flag a pending greeting.
    pub fn enter_sleep(
        &mut self,
        renderer: &mut Renderer,
        buttons: &mut Buttons,
        ctx: &mut GameContext,
        scene_manager: &mut SceneManager,
        time_system: &mut TimeSystem,
    ) -> SleepExit {
        println!("[Sleep] Entering basic sleep");
        self.sleeping = true;
        renderer.power_off();

        // Wait until all buttons have been released for a stable period
        // before arming the wake check. Without this, the A-press that
        // triggered an explicit sleep (or its release bounce) is read as
        // an instant wake.
        wait_buttons_stable_released(buttons, Duration::from_millis(500));

        let mut last_tick = Instant::now();
        while !buttons.any_pressed() {
            watchdog::feed();
            let elapsed_ms = last_tick.elapsed().as_millis();
            if elapsed_ms >= SLEEP_FRAME_TIME_MS {
                last_tick = Instant::now();
                let dt = (elapsed_ms as f32) / 1000.0 * ctx.time_speed;
                time_system.advance(ctx, dt);
                scene_manager.sleep_update(ctx, buttons, dt);
                if deep_sleep_due(power::status()) {
                    println!("[Sleep] Battery low, leaving basic sleep for deep sleep");
                    self.sleeping = false;
                    return SleepExit::DeepSleep;
                }
            }
        }

        println!("[Sleep] Waking from basic sleep");
        self.sleeping = false;
        self.last_activity = Instant::now();
        renderer.power_on();
        // Drop the wake press so it doesn't fire a game action.
        buttons.consume_all();
        // Greet the returning player on the next behavior pick.
        ctx.pending_wake_greeting = true;
        // Tell the active behavior to wind down fast. Sleeping / napping
        // either honor this (rouse in ~3s) or veto the greeting and stay
        // asleep based on serenity (matches the MicroPython port).
        scene_manager.mark_behavior_almost_done(ctx);
        SleepExit::Woken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(pct: Option<u8>, on_usb: bool) -> Option<PowerStatus> {
        Some(PowerStatus { battery_pct: pct, charging: on_usb, on_usb })
    }

    #[test]
    fn deep_sleep_only_on_a_low_battery_without_usb() {
        assert!(deep_sleep_due(battery(Some(DEEP_SLEEP_BATTERY_PCT), false)));
        assert!(deep_sleep_due(battery(Some(0), false)));
        assert!(!deep_sleep_due(battery(Some(DEEP_SLEEP_BATTERY_PCT + 1), false)));
        assert!(!deep_sleep_due(battery(Some(1), true)), "never on USB");
        assert!(!deep_sleep_due(battery(None, false)), "no battery");
        assert!(!deep_sleep_due(None), "no PMIC reading");
    }
}
