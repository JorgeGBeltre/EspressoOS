#![allow(dead_code)]

use core::sync::atomic::{AtomicUsize, Ordering};

use esp_hal::handler;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::timer::{AnyTimer, PeriodicTimer};
use esp_hal::Blocking;

use super::sync::Mutex;

pub const TICK_HZ: u32 = 100;

type SchedTimer = PeriodicTimer<'static, Blocking>;

static PERIODIC: Mutex<Option<SchedTimer>> = Mutex::new(None);

static TICK_HANDLER: AtomicUsize = AtomicUsize::new(0);

pub fn set_tick_handler(handler: fn()) {
    TICK_HANDLER.store(handler as usize, Ordering::SeqCst);
}

#[inline(always)]
fn invoke_tick_handler() {
    let ptr = TICK_HANDLER.load(Ordering::SeqCst);
    if ptr != 0 {

        let f: fn() = unsafe { core::mem::transmute::<usize, fn()>(ptr) };
        f();
    } else {

        crate::scheduler::tick();
    }
}

#[handler]
fn systimer_tick_isr() {

    if let Some(t) = PERIODIC.lock().as_mut() {
        t.clear_interrupt();
    }

    invoke_tick_handler();
}

pub fn init() {

    if PERIODIC.lock().is_some() {
        return;
    }

    let systimer_perif = unsafe { esp_hal::peripherals::SYSTIMER::steal() };
    let systimer = SystemTimer::new(systimer_perif);

    let alarm: AnyTimer = systimer.alarm0.into();
    let mut periodic = PeriodicTimer::new(alarm);

    periodic.set_interrupt_handler(systimer_tick_isr);

    let periodo_us: u64 = 1_000_000u64 / TICK_HZ as u64;

    let _ = periodic.start(esp_hal::time::Duration::from_ticks(periodo_us));

    {
        let mut g = PERIODIC.lock();
        *g = Some(periodic);
        if let Some(t) = g.as_mut() {
            t.enable_interrupt(true);
        }
    }

}

pub fn uptime_ms() -> u64 {

    esp_hal::time::now().duration_since_epoch().to_millis()
}
