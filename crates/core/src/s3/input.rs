//! Touch + BOOT/USER as the 8 button "pins". Same trick as the desktop
//! build: `Buttons` keeps calling `ButtonPin::is_low()`, which here reads a
//! shared mask instead of a GPIO.
//!
//! The mask is refreshed lazily from `is_low`, at most every `POLL_MS`. That
//! covers `Game::tick` and also the sleep loops that spin on
//! `buttons.any_pressed()` without ticking, with no hook in game code.
//!
//! The touch INT line only pulses, so a GPIO interrupt latches its falling
//! edge, and `TouchTracker` keeps polling until the finger lifts. A slow idle
//! poll covers a missed edge.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use catode32_amoled216::{
    cst9220::{Cst9220, BOARD_ORIENTATION, MAX_POINTS},
    touch::{key_mask, TouchTracker},
};
use critical_section::Mutex;
use esp_hal::{
    gpio::{Event, Input, Io, Output},
    handler,
    peripherals::IO_MUX,
};

use super::board::SharedI2c;
use crate::platform::time::{Duration, Instant};

/// Fastest touch/key refresh. Well under the 50 ms button debounce.
const POLL_MS: u64 = 20;
/// Without an INT edge, still read the touch chip this often.
const IDLE_POLL_MS: u64 = 500;

type Touch = Cst9220<SharedI2c, Output<'static>>;

struct State {
    touch: Touch,
    tracker: TouchTracker,
    boot: Input<'static>,
    user: Input<'static>,
    last_poll: Option<Instant>,
    last_idle_poll: Instant,
    /// Set by `sleep_touch`: only the keys are read.
    touch_asleep: bool,
    read_error_logged: bool,
}

/// Bit per button, in `input::Buttons` order. Set = pressed.
static MASK: AtomicU8 = AtomicU8::new(0);
static INT_EDGE: AtomicBool = AtomicBool::new(false);
static INT_PIN: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));
static STATE: Mutex<RefCell<Option<State>>> = Mutex::new(RefCell::new(None));

pub struct ButtonPin {
    bit: u8,
}

impl ButtonPin {
    pub fn new(idx: usize) -> Self {
        Self { bit: 1 << idx }
    }

    pub fn is_low(&self) -> bool {
        refresh();
        MASK.load(Ordering::Relaxed) & self.bit != 0
    }
}

/// Everything currently pressed (touch and keys), for the controls highlight.
pub fn pressed_mask() -> u8 {
    MASK.load(Ordering::Relaxed)
}

/// Called once from `board::init`.
pub(crate) fn install(io_mux: IO_MUX<'static>, touch: Touch, mut int: Input<'static>, boot: Input<'static>, user: Input<'static>) {
    let mut io = Io::new(io_mux);
    io.set_interrupt_handler(gpio_handler);
    critical_section::with(|cs| {
        int.listen(Event::FallingEdge);
        INT_PIN.borrow_ref_mut(cs).replace(int);
        STATE.borrow_ref_mut(cs).replace(State {
            touch,
            tracker: TouchTracker::new(),
            boot,
            user,
            last_poll: None,
            last_idle_poll: Instant::now(),
            touch_asleep: false,
            read_error_logged: false,
        });
    });
    // `Io` only registers the handler; dropping it doesn't undo that.
}

/// Put the touch chip to sleep (deep-sleep stand-in). From here on only
/// BOOT and USER are read. A reset wakes the chip again.
pub fn sleep_touch() {
    with_state(|s| {
        if let Err(e) = s.touch.sleep() {
            println!("[s3] touch sleep failed: {:?}", e);
        }
        s.touch_asleep = true;
        s.tracker = TouchTracker::new();
    });
    refresh_now();
}

#[handler]
fn gpio_handler() {
    critical_section::with(|cs| {
        if let Some(pin) = INT_PIN.borrow_ref_mut(cs).as_mut() {
            if pin.is_interrupt_set() {
                pin.clear_interrupt();
                INT_EDGE.store(true, Ordering::Relaxed);
            }
        }
    });
}

/// Take the state out of the mutex for the I2C read, so interrupts aren't
/// held off during it. Single-threaded use only (the game loop).
fn with_state(f: impl FnOnce(&mut State)) {
    let Some(mut s) = critical_section::with(|cs| STATE.borrow_ref_mut(cs).take()) else {
        return;
    };
    f(&mut s);
    critical_section::with(|cs| STATE.borrow_ref_mut(cs).replace(s));
}

fn refresh() {
    with_state(|s| {
        if s.last_poll.is_some_and(|t| t.elapsed() < Duration::from_millis(POLL_MS)) {
            return;
        }
        poll(s);
    });
}

fn refresh_now() {
    with_state(poll);
}

fn poll(s: &mut State) {
    s.last_poll = Some(Instant::now());
    let mut mask = key_mask(s.boot.is_low(), s.user.is_low());
    if !s.touch_asleep {
        mask |= poll_touch(s);
    }
    MASK.store(mask, Ordering::Relaxed);
}

fn poll_touch(s: &mut State) -> u8 {
    if INT_EDGE.swap(false, Ordering::Relaxed) {
        s.tracker.on_int_edge();
    }
    let idle_poll = !s.tracker.should_poll() && s.last_idle_poll.elapsed() >= Duration::from_millis(IDLE_POLL_MS);
    if !s.tracker.should_poll() && !idle_poll {
        return s.tracker.mask();
    }
    if idle_poll {
        s.last_idle_poll = Instant::now();
    }

    let mut raw = [(0u16, 0u16); MAX_POINTS];
    let n = match s.touch.read_points(&mut raw) {
        Ok(n) => n,
        Err(e) => {
            if !s.read_error_logged {
                s.read_error_logged = true;
                println!("[s3] touch read failed: {:?} (logged once)", e);
            }
            0
        }
    };
    if idle_poll {
        if n == 0 {
            return s.tracker.mask();
        }
        s.tracker.on_int_edge();
    }
    let mut points = [(0u16, 0u16); MAX_POINTS];
    for (p, r) in points.iter_mut().zip(&raw[..n]) {
        *p = BOARD_ORIENTATION.map(*r);
    }
    s.tracker.update(&points[..n])
}
