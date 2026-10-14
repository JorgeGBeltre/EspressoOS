#![allow(dead_code)]

use crate::arch::xtensa::Mutex;
use crate::prelude::*;
use core::time::Duration;
use esp_hal::rtc_cntl::sleep::TimerWakeupSource;
use esp_hal::rtc_cntl::Rtc;

static RTC: Mutex<Option<Rtc<'static>>> = Mutex::new(None);

pub fn init(lpwr: esp_hal::peripherals::LPWR) {
    let rtc = Rtc::new(lpwr);
    crate::arch::xtensa::interrupts::critical_section(|| {
        *RTC.lock() = Some(rtc);
    });
}

pub fn enter_deep_sleep(seconds: u64) -> ! {
    esp_println::println!(
        "[power] Entering Deep Sleep for {} seconds (reboot on wakeup)...",
        seconds
    );
    let timer = TimerWakeupSource::new(Duration::from_secs(seconds));
    let mut rtc = match RTC.lock().take() {
        Some(r) => r,
        None => {
            esp_println::println!("[power] ERROR: RTC not initialized");
            loop {
                core::hint::spin_loop();
            }
        }
    };
    rtc.sleep_deep(&[&timer]);
}

pub fn enter_light_sleep(seconds: u64) {
    esp_println::println!(
        "[power] Entering low-power CPU sleep (WFI) for {} seconds...",
        seconds
    );
    let start = crate::arch::xtensa::timer::uptime_ms();
    let target = seconds.saturating_mul(1000);
    while (crate::arch::xtensa::timer::uptime_ms().wrapping_sub(start) as u64) < target {
        unsafe {
            core::arch::asm!("waiti 0");
        }
    }
}

// ---- /dev/power: sleep / deep-sleep / reboot / idle por ioctl ----

pub const POWER_SLEEP: u32 = 0;
pub const POWER_DEEP_SLEEP: u32 = 1;
pub const POWER_REBOOT: u32 = 2;
pub const POWER_IDLE: u32 = 3;

struct PowerDevice;

impl crate::vfs::devfs::Device for PowerDevice {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            POWER_SLEEP => {
                enter_light_sleep(arg as u64);
                Ok(0)
            }
            POWER_DEEP_SLEEP => enter_deep_sleep(arg as u64),
            POWER_REBOOT => {
                esp_hal::reset::software_reset();
                loop {
                    core::hint::spin_loop();
                }
            }
            POWER_IDLE => {
                unsafe {
                    core::arch::asm!("waiti 0");
                }
                Ok(0)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(PowerDevice)
}
