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

// ---- /dev/power: deep-sleep / reboot por ioctl (SP2 R5). D-5: cero syscalls. ----

// POWER_SLEEP (0) removida (SP2 R6): esp-hal's `rtc.sleep_light()` cuelga la CPU en este
// hardware/generación de HAL -- no es un bug de este kernel, es una limitación de
// plataforma (ver README §9, punto 1 histórico). Deep sleep y reboot cubren los mismos
// casos de uso (ahorro de energía, liberar el bus) sin el riesgo de colgar la placa, así
// que en vez de dejar un camino que cuelga el CPU, el ioctl 0 ahora devuelve NotSupported.
// El número 0 se deja sin reasignar para no romper binarios de userland ya compilados que
// aún lo prueben.
const POWER_SLEEP_REMOVED: u32 = 0;
pub const POWER_DEEP_SLEEP: u32 = 1;
pub const POWER_REBOOT: u32 = 2;

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
            // `arg` = segundos. deep-sleep y reboot no vuelven (la placa reinicia).
            POWER_SLEEP_REMOVED => Err(KError::NotSupported),
            POWER_DEEP_SLEEP => enter_deep_sleep(arg as u64),
            POWER_REBOOT => {
                esp_hal::reset::software_reset();
                loop {
                    core::hint::spin_loop();
                }
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(PowerDevice)
}
