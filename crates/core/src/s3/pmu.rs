//! AXP2101 battery/USB state behind `platform::power::status`. Read at most
//! every `POLL_MS` (the gauge moves slowly) and cached in between, so
//! callers can ask every frame. Like `input`, the driver is taken out of
//! its mutex for the I2C read so interrupts aren't held off during it.

use core::cell::RefCell;

use catode32_amoled216::axp2101::{Axp2101, Status};
use critical_section::Mutex;

use super::board::SharedI2c;
use crate::platform::power::PowerStatus;
use crate::platform::time::{Duration, Instant};

const POLL_MS: u64 = 5_000;

struct State {
    pmu: Axp2101<SharedI2c>,
    last: Option<PowerStatus>,
    last_poll: Option<Instant>,
    read_error_logged: bool,
}

static STATE: Mutex<RefCell<Option<State>>> = Mutex::new(RefCell::new(None));

/// Apply the demo settings and start polling. On failure the status stays
/// `None` (the status bar shows "--").
pub(crate) fn install(mut pmu: Axp2101<SharedI2c>) {
    if let Err(e) = pmu.init() {
        println!("[s3] PMU init failed: {:?}", e);
        return;
    }
    let mut state = State { pmu, last: None, last_poll: None, read_error_logged: false };
    poll(&mut state);
    if let Some(s) = state.last {
        println!("[s3] PMU ok: {:?}", s);
    }
    critical_section::with(|cs| STATE.borrow_ref_mut(cs).replace(state));
}

pub fn status() -> Option<PowerStatus> {
    let mut state = critical_section::with(|cs| STATE.borrow_ref_mut(cs).take())?;
    if state.last_poll.is_none_or(|t| t.elapsed() >= Duration::from_millis(POLL_MS)) {
        poll(&mut state);
    }
    let last = state.last;
    critical_section::with(|cs| STATE.borrow_ref_mut(cs).replace(state));
    last
}

fn poll(s: &mut State) {
    s.last_poll = Some(Instant::now());
    match s.pmu.status() {
        Ok(st) => s.last = Some(to_power(st)),
        Err(e) => {
            if !s.read_error_logged {
                s.read_error_logged = true;
                println!("[s3] PMU read failed: {:?} (logged once)", e);
            }
            s.last = None;
        }
    }
}

fn to_power(s: Status) -> PowerStatus {
    PowerStatus { battery_pct: s.battery_pct, charging: s.charging(), on_usb: s.vbus_in }
}
