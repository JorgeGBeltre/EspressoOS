#![allow(dead_code)]

use core::arch::asm;
use esp_hal::xtensa_lx_rt::exception::Context as ExceptionContext;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {



    pub frame: ExceptionContext,
}

const STACK_ALIGN_MASK: usize = 0xF;


const PS_UM: u32 = 1 << 5;
const PS_WOE: u32 = 1 << 18;
const PS_CALLINC1: u32 = 1 << 16;

#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize, is_user: bool) -> Context {


    let top = ((stack_top as usize) & !STACK_ALIGN_MASK) as u32;

    let mut frame = ExceptionContext::default();
    frame.PC = entry as usize as u32;
    frame.PS = if is_user {
        PS_UM | PS_WOE | PS_CALLINC1
    } else {
        PS_WOE | PS_CALLINC1
    };
    frame.A0 = 0;
    frame.A1 = top;
    frame.A6 = arg as u32;

    Context { frame }
}

#[inline(always)]
pub unsafe fn resume_task(sp: u32) -> ! {
    asm!(

        "mov a1, {0}",
        

        "movi a4, 1",
        "rsr.windowbase a5",
        "ssl a5",
        "sll a4, a4",
        "wsr.windowstart a4",
        "rsync",
        

        "movi a0, restore_context",
        "callx0 a0",
        

        "l32i a0, a1, 4",
        "wsr a0, PS",
        "l32i a0, a1, 0",
        "wsr a0, EPC1",
        "rsync",
        

        "l32i a0, a1, 8",
        "l32i a1, a1, 12",
        "rsync",
        

        "rfe",
        in(reg) sp,
        options(noreturn)
    );
}
