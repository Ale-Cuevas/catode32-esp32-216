//! Software reset / process exit. Firmware drops into the chip's reset path;
//! desktop exits the process.

#[cfg(not(feature = "desktop"))]
pub fn software_reset() -> ! {
    esp_hal::system::software_reset()
}

#[cfg(feature = "desktop")]
pub fn software_reset() -> ! {
    std::process::exit(0)
}

/// Battery and USB state, on boards that can tell (the S3's AXP2101).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerStatus {
    /// Fuel gauge 0..=100, `None` without a battery.
    pub battery_pct: Option<u8>,
    pub charging: bool,
    pub on_usb: bool,
}

/// Latest reading, `None` when the board has no PMIC or it can't be read.
/// Cheap: the S3 polls the PMIC at most every few seconds.
#[cfg(feature = "s3")]
pub fn status() -> Option<PowerStatus> {
    crate::s3::pmu::status()
}

#[cfg(all(not(feature = "desktop"), not(feature = "s3")))]
pub fn status() -> Option<PowerStatus> {
    None
}

#[cfg(feature = "desktop")]
mod desktop_status {
    use super::PowerStatus;
    use std::sync::Mutex;

    static STATUS: Mutex<Option<PowerStatus>> = Mutex::new(None);

    pub fn status() -> Option<PowerStatus> {
        *STATUS.lock().unwrap()
    }

    /// The simulator's stand-in for the PMIC (`--amoled` keys).
    pub fn set_desktop_status(s: Option<PowerStatus>) {
        *STATUS.lock().unwrap() = s;
    }
}

#[cfg(feature = "desktop")]
pub use desktop_status::{set_desktop_status, status};
