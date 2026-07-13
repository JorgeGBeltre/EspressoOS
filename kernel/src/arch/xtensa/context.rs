#![allow(dead_code, unused_imports)]

use core::arch::asm;
use core::mem::offset_of;

use crate::prelude::*;

const PS_INTLEVEL_MASK: u32 = 0x0000_000F;

const PS_EXCM: u32 = 1 << 4;

const PS_UM: u32 = 1 << 5;

const PS_CALLINC1: u32 = 1 << 16;

const PS_WOE: u32 = 1 << 18;

const XCHAL_EXCM_LEVEL: u32 = 3;

const FRAME_INITIAL_PS: u32 = PS_UM | PS_EXCM | PS_WOE | PS_CALLINC1;

const INITIAL_PS: u32 = PS_UM | PS_WOE;

const STACK_ALIGN_MASK: usize = 0xF;

const XT_STK_EXIT: usize = 0x00;

const XT_STK_PC: usize = 0x04;

const XT_STK_PS: usize = 0x08;

const XT_STK_A0: usize = 0x0C;

const XT_STK_A1: usize = 0x10;

const XT_STK_A6: usize = 0x24;

const XT_STK_SAR: usize = 0x4C;

const XT_STK_FRMSZ: usize = 0x50;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Context {

    pub ps: u32,

    pub sp: u32,

    pub a0: u32,

    pub first_run: u32,
}

#[inline(always)]
unsafe fn frame_put(base: usize, off: usize, val: u32) {

    unsafe { ((base + off) as *mut u32).write_volatile(val) };
}

#[inline(never)]
pub fn init_task_stack(stack_top: *mut u8, entry: fn(usize), arg: usize) -> Context {

    let top = (stack_top as usize) & !STACK_ALIGN_MASK;

    let frame_base = top.saturating_sub(XT_STK_FRMSZ) & !STACK_ALIGN_MASK;

    let a1_physical = frame_base + XT_STK_FRMSZ;

    let entry_addr = entry as usize as u32;

    unsafe {
        let words = XT_STK_FRMSZ / 4;
        let mut i = 0;
        while i < words {
            frame_put(frame_base, i * 4, 0);
            i += 1;
        }
        frame_put(frame_base, XT_STK_EXIT, 0);
        frame_put(frame_base, XT_STK_PC, entry_addr);
        frame_put(frame_base, XT_STK_PS, FRAME_INITIAL_PS);
        frame_put(frame_base, XT_STK_A0, 0);
        frame_put(frame_base, XT_STK_A1, a1_physical as u32);
        frame_put(frame_base, XT_STK_A6, arg as u32);
        frame_put(frame_base, XT_STK_SAR, 0);
    }

    Context {
        ps: FRAME_INITIAL_PS,
        sp: frame_base as u32,
        a0: 0,
        first_run: 1,
    }
}

#[inline(never)]
pub unsafe fn switch_to(current: *mut Context, next: *const Context) {

    unsafe {
        asm!(

            "rsr.ps  a4",
            "s32i    a4, a2, {O_PS}",
            "s32i    a0, a2, {O_A0}",
            "s32i    a1, a2, {O_SP}",
            "movi    a4, 0",
            "s32i    a4, a2, {O_FIRST}",

            "rsr.ps  a8",
            "extui   a5, a8, 0, 4",
            "bgeui   a5, 3, 1f",
            "movi    a5, 3",
            "1:",

            "movi    a4, 1",
            "slli    a6, a4, 18",
            "slli    a4, a4, 5",
            "or      a4, a4, a6",
            "or      a5, a5, a4",
            "wsr.ps  a5",
            "rsync",

            "rsr.epc1 a0",

            "and a12, a12, a12",
            "rotw 3",
            "and a12, a12, a12",
            "rotw 3",
            "and a12, a12, a12",
            "rotw 3",
            "and a12, a12, a12",
            "rotw 3",
            "and a12, a12, a12",
            "rotw 4",
            "wsr.epc1 a0",
            "rsync",

            "l32i    a4, a3, {O_FIRST}",
            "bnez    a4, 2f",

            "l32i    a4, a3, {O_PS}",
            "l32i    a0, a3, {O_A0}",
            "l32i    a1, a3, {O_SP}",
            "wsr.ps  a4",
            "rsync",

            "movi    a4, 1",
            "rsr.windowbase a5",
            "ssl     a5",
            "sll     a4, a4",
            "wsr.windowstart a4",
            "rsync",

            "retw",

            "2:",
            "l32i    a1, a3, {O_SP}",
            "l32i    a5, a1, {XT_PS}",
            "wsr.ps  a5",
            "l32i    a5, a1, {XT_PC}",
            "wsr.epc1 a5",
            "rsync",

            "movi    a4, 1",
            "rsr.windowbase a5",
            "ssl     a5",
            "sll     a4, a4",
            "wsr.windowstart a4",
            "rsync",

            "movi    a4, 0",
            "l32i    a6, a1, {XT_A6}",
            "l32i    a0, a1, {XT_A0}",
            "l32i    a1, a1, {XT_A1}",
            "rfe",

            O_PS    = const offset_of!(Context, ps),
            O_SP    = const offset_of!(Context, sp),
            O_A0    = const offset_of!(Context, a0),
            O_FIRST = const offset_of!(Context, first_run),
            XT_PS   = const XT_STK_PS,
            XT_PC   = const XT_STK_PC,
            XT_A0   = const XT_STK_A0,
            XT_A1   = const XT_STK_A1,
            XT_A6   = const XT_STK_A6,
            in("a2") current,
            in("a3") next,
            options(noreturn),
        )
    }
}
