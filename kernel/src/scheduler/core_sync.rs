#![allow(dead_code)]



















use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static SMP_RUNNING: AtomicBool = AtomicBool::new(false);

static CORE1_TICKS: AtomicU32 = AtomicU32::new(0);


pub fn is_running() -> bool {
    SMP_RUNNING.load(Ordering::Relaxed)
}


pub fn core1_ticks() -> u64 {
    CORE1_TICKS.load(Ordering::Relaxed) as u64
}


pub fn current_core_id() -> usize {
    match esp_hal::Cpu::current() {
        esp_hal::Cpu::ProCpu => 0,
        esp_hal::Cpu::AppCpu => 1,
    }
}

#[cfg(feature = "smp")]
mod imp {
    use super::*;
    use esp_hal::cpu_control::{CpuControl, Stack};
    use esp_hal::peripherals::CPU_CTRL;
    use esp_println::println;

    const APP_STACK_SIZE: usize = 8 * 1024;
    static mut APP_CORE_STACK: Stack<APP_STACK_SIZE> = Stack::new();

    pub fn start(cpu_ctrl: CPU_CTRL) {
        let mut cpu_control = CpuControl::new(cpu_ctrl);

        let stack: &'static mut Stack<APP_STACK_SIZE> =
            unsafe { &mut *core::ptr::addr_of_mut!(APP_CORE_STACK) };

        match cpu_control.start_app_core(stack, app_core_main) {
            Ok(guard) => {

                core::mem::forget(guard);
                SMP_RUNNING.store(true, Ordering::Release);
                println!("[smp] APP_CPU (core 1) started");
            }
            Err(_) => println!("[smp] ERROR: failed to start the APP_CPU"),
        }
    }



    fn app_core_main() {
        crate::scheduler::run_secondary();
    }
}




#[cfg(feature = "smp")]
pub fn worker_entry(_arg: usize) {
    use esp_println::println;
    let mut last = 0u64;
    loop {
        let now = crate::arch::xtensa::timer::uptime_ms();
        if now.wrapping_sub(last) >= 1000 {
            last = now;
            let t = CORE1_TICKS.fetch_add(1, Ordering::AcqRel) + 1;
            println!(
                "[smp] worker task on core{} tick={} uptime={}ms",
                current_core_id(),
                t,
                now
            );
        }
        crate::scheduler::yield_now();
    }
}



#[cfg(feature = "smp")]
pub fn start_secondary_core(cpu_ctrl: esp_hal::peripherals::CPU_CTRL) {
    imp::start(cpu_ctrl);
}
