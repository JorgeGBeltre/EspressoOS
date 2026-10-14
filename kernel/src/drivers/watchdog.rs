#![allow(dead_code)]

use crate::prelude::*;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

extern "C" {
    fn rtc_get_reset_reason(cpu_no: u32) -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    PowerOn,
    SoftwareReset,
    DeepSleep,
    WatchdogRtc,
    WatchdogTimerGroup0,
    WatchdogTimerGroup1,
    BrownOut,
    Other(u32),
}

impl ResetReason {
    pub fn from_raw(code: u32) -> Self {
        match code {
            1 => ResetReason::PowerOn,
            3 => ResetReason::SoftwareReset,
            4 | 9 | 13 | 16 => ResetReason::WatchdogRtc,
            5 => ResetReason::DeepSleep,
            7 | 11 => ResetReason::WatchdogTimerGroup0,
            8 => ResetReason::WatchdogTimerGroup1,
            15 => ResetReason::BrownOut,
            other => ResetReason::Other(other),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ResetReason::PowerOn => "Power-on Reset",
            ResetReason::SoftwareReset => "Software Reset",
            ResetReason::DeepSleep => "Deep Sleep Wakeup",
            ResetReason::WatchdogRtc => "RTC Watchdog Reset",
            ResetReason::WatchdogTimerGroup0 => "Timer Group 0 Watchdog Reset",
            ResetReason::WatchdogTimerGroup1 => "Timer Group 1 Watchdog Reset",
            ResetReason::BrownOut => "Brown-out Detector Reset",
            ResetReason::Other(_) => "Other / Unknown Reset",
        }
    }
}

static FEEDS_COUNT: AtomicU32 = AtomicU32::new(0);
static WDT_ENABLED: AtomicBool = AtomicBool::new(true);
static WDT_TIMEOUT_SECS: AtomicU32 = AtomicU32::new(10);

pub fn init() {
    let reason = get_reset_reason();
    esp_println::println!("[watchdog] Initialized. Boot reset reason: {}", reason.as_str());
}

pub fn get_reset_reason() -> ResetReason {
    let raw = unsafe { rtc_get_reset_reason(0) };
    ResetReason::from_raw(raw)
}

pub fn feed() {
    if WDT_ENABLED.load(Ordering::Relaxed) {
        FEEDS_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn feeds_count() -> u32 {
    FEEDS_COUNT.load(Ordering::Relaxed)
}

pub fn is_enabled() -> bool {
    WDT_ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(enabled: bool) {
    WDT_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn timeout_secs() -> u32 {
    WDT_TIMEOUT_SECS.load(Ordering::Relaxed)
}

pub fn set_timeout(secs: u32) {
    WDT_TIMEOUT_SECS.store(secs, Ordering::Relaxed);
}

// IOCTL command codes for /dev/watchdog
pub const WDT_IOC_FEED: u32 = 0x5701;
pub const WDT_IOC_GET_STATUS: u32 = 0x5702;
pub const WDT_IOC_SET_TIMEOUT: u32 = 0x5703;
pub const WDT_IOC_ENABLE: u32 = 0x5704;

pub struct WatchdogDevice;

impl crate::vfs::devfs::Device for WatchdogDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let reason = get_reset_reason();
        let status = alloc::format!(
            "watchdog: {}\ntimeout: {}s\nfeeds: {}\nreset_reason: {}\n",
            if is_enabled() { "enabled" } else { "disabled" },
            timeout_secs(),
            feeds_count(),
            reason.as_str()
        );
        let bytes = status.as_bytes();
        let off = off as usize;
        if off >= bytes.len() {
            return Ok(0);
        }
        let avail = &bytes[off..];
        let n = buf.len().min(avail.len());
        buf[..n].copy_from_slice(&avail[..n]);
        Ok(n)
    }

    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        // Writing any byte to /dev/watchdog feeds ("pings") the watchdog
        feed();
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            WDT_IOC_FEED => {
                feed();
                Ok(0)
            }
            WDT_IOC_GET_STATUS => {
                Ok(if is_enabled() { 1 } else { 0 })
            }
            WDT_IOC_SET_TIMEOUT => {
                if arg == 0 || arg > 3600 {
                    return Err(KError::InvalidArgument);
                }
                set_timeout(arg as u32);
                Ok(0)
            }
            WDT_IOC_ENABLE => {
                set_enabled(arg != 0);
                Ok(0)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(WatchdogDevice)
}
