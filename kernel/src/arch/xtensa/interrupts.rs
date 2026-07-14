#![allow(dead_code)]

use core::arch::asm;

const CRITICAL_INTLEVEL: u32 = 15;

pub fn init() {

    let _vecbase_actual = read_vecbase();

}

extern "C" {
    fn handle_interrupts(level: u32, save_frame: &mut esp_hal::xtensa_lx_rt::exception::Context);
}

#[no_mangle]
#[link_section = ".rwtext"]
unsafe extern "C" fn __level_1_interrupt(level: u32, save_frame: &mut esp_hal::xtensa_lx_rt::exception::Context) {
    handle_interrupts(level, save_frame);




    if crate::scheduler::need_resched() {
        crate::scheduler::preempt_switch(save_frame);
    }
    let _ = crate::scheduler::process::check_signals(save_frame);
}

#[inline(always)]
pub fn disable() -> u32 {
    let ps_previo: u32;

    unsafe {
        asm!("rsil {0}, 15", out(reg) ps_previo, options(nostack));
    }
    ps_previo
}

#[inline(always)]
pub fn restore(state: u32) {
    unsafe {
        asm!(
            "wsr.ps {0}",
            "rsync",
            in(reg) state,
            options(nostack),
        );
    }
}

#[inline]
pub fn critical_section<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let estado = disable();
    let resultado = f();
    restore(estado);
    resultado
}

#[inline(always)]
fn read_vecbase() -> u32 {
    let vecbase: u32;

    unsafe {
        asm!("rsr.vecbase {0}", out(reg) vecbase, options(nostack, nomem, preserves_flags));
    }
    vecbase
}
