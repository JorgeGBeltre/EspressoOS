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

pub fn enter_light_sleep(seconds: u64) {
    esp_println::println!("[power] Entering Light Sleep for {} seconds...", seconds);
    crate::arch::xtensa::interrupts::critical_section(|| {
        let mut guard = RTC.lock();
        if let Some(rtc) = guard.as_mut() {
            let timer = TimerWakeupSource::new(Duration::from_secs(seconds));
            rtc.sleep_light(&[&timer]);
        }
    });
    esp_println::println!("[power] Light Sleep wakeup!");
}

pub fn enter_deep_sleep(seconds: u64) -> ! {
    esp_println::println!(
        "[power] Entering Deep Sleep for {} seconds (reboot on wakeup)...",
        seconds
    );
    crate::arch::xtensa::interrupts::critical_section(|| {
        let mut guard = RTC.lock();
        if let Some(rtc) = guard.as_mut() {
            let timer = TimerWakeupSource::new(Duration::from_secs(seconds));
            rtc.sleep_deep(&[&timer]);
        }
    });
    loop {
        core::hint::spin_loop();
    }
}
