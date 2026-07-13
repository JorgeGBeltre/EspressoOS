#![allow(dead_code)]

use crate::prelude::*;
use crate::arch::xtensa::Mutex;
use esp_hal::rtc_cntl::Rtc;
use esp_hal::rtc_cntl::sleep::TimerWakeupSource;
use core::time::Duration;

static RTC: Mutex<Option<Rtc<'static>>> = Mutex::new(None);

pub fn init(lpwr: esp_hal::peripherals::LPWR) {
    let rtc = Rtc::new(lpwr);
    crate::arch::xtensa::interrupts::critical_section(|| {
        *RTC.lock() = Some(rtc);
    });
}

pub fn enter_light_sleep(seconds: u64) {
    esp_println::println!("[power] Entrando en Light Sleep por {} segundos...", seconds);
    crate::arch::xtensa::interrupts::critical_section(|| {
        let mut guard = RTC.lock();
        if let Some(rtc) = guard.as_mut() {
            let timer = TimerWakeupSource::new(Duration::from_secs(seconds));
            rtc.sleep_light(&[&timer]);
        }
    });
    esp_println::println!("[power] Wakeup de Light Sleep!");
}

pub fn enter_deep_sleep(seconds: u64) -> ! {
    esp_println::println!("[power] Entrando en Deep Sleep por {} segundos (reinicio al despertar)...", seconds);
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
