#![allow(dead_code)]

use crate::prelude::*;
use core::sync::atomic::{AtomicU32, Ordering};

pub enum PinMode {
    Input,
    Output,
}

const GPIO_BASE: usize = 0x6000_4000;

const GPIO_OUT_REG: usize = GPIO_BASE + 0x0004;
const GPIO_OUT_W1TS: usize = GPIO_BASE + 0x0008;
const GPIO_OUT_W1TC: usize = GPIO_BASE + 0x000C;
const GPIO_ENABLE_W1TS: usize = GPIO_BASE + 0x0024;
const GPIO_ENABLE_W1TC: usize = GPIO_BASE + 0x0028;
const GPIO_IN_REG: usize = GPIO_BASE + 0x003C;

const GPIO_OUT1_REG: usize = GPIO_BASE + 0x0010;
const GPIO_OUT1_W1TS: usize = GPIO_BASE + 0x0014;
const GPIO_OUT1_W1TC: usize = GPIO_BASE + 0x0018;
const GPIO_ENABLE1_W1TS: usize = GPIO_BASE + 0x0030;
const GPIO_ENABLE1_W1TC: usize = GPIO_BASE + 0x0034;
const GPIO_IN1_REG: usize = GPIO_BASE + 0x0040;

const GPIO_FUNC_OUT_SEL_BASE: usize = GPIO_BASE + 0x0554;

const SIG_GPIO_OUT_IDX: u32 = 128;

const MAX_GPIO: u8 = 48;

static OUTPUT_MASK_LO: AtomicU32 = AtomicU32::new(0);
static OUTPUT_MASK_HI: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
unsafe fn reg_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn reg_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val)
}

fn check_pin(pin: u8) -> KResult<()> {
    if pin > MAX_GPIO {
        Err(KError::InvalidArgument)
    } else {
        Ok(())
    }
}

fn is_output(pin: u8) -> bool {
    if pin < 32 {
        OUTPUT_MASK_LO.load(Ordering::Relaxed) & (1u32 << pin) != 0
    } else {
        OUTPUT_MASK_HI.load(Ordering::Relaxed) & (1u32 << (pin - 32)) != 0
    }
}

fn mark_output(pin: u8, is_out: bool) {
    if pin < 32 {
        let bit = 1u32 << pin;
        if is_out {
            OUTPUT_MASK_LO.fetch_or(bit, Ordering::Relaxed);
        } else {
            OUTPUT_MASK_LO.fetch_and(!bit, Ordering::Relaxed);
        }
    } else {
        let bit = 1u32 << (pin - 32);
        if is_out {
            OUTPUT_MASK_HI.fetch_or(bit, Ordering::Relaxed);
        } else {
            OUTPUT_MASK_HI.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

fn set_enable(pin: u8, enable: bool) {
    if pin < 32 {
        let reg = if enable {
            GPIO_ENABLE_W1TS
        } else {
            GPIO_ENABLE_W1TC
        };
        unsafe { reg_write(reg, 1u32 << pin) };
    } else {
        let reg = if enable {
            GPIO_ENABLE1_W1TS
        } else {
            GPIO_ENABLE1_W1TC
        };
        unsafe { reg_write(reg, 1u32 << (pin - 32)) };
    }
}

fn set_level(pin: u8, high: bool) {
    if pin < 32 {
        let reg = if high { GPIO_OUT_W1TS } else { GPIO_OUT_W1TC };
        unsafe { reg_write(reg, 1u32 << pin) };
    } else {
        let reg = if high { GPIO_OUT1_W1TS } else { GPIO_OUT1_W1TC };
        unsafe { reg_write(reg, 1u32 << (pin - 32)) };
    }
}

fn read_output_latch(pin: u8) -> bool {
    if pin < 32 {
        (unsafe { reg_read(GPIO_OUT_REG) } & (1u32 << pin)) != 0
    } else {
        (unsafe { reg_read(GPIO_OUT1_REG) } & (1u32 << (pin - 32))) != 0
    }
}

pub fn configure(pin: u8, mode: PinMode) -> KResult<()> {
    check_pin(pin)?;
    match mode {
        PinMode::Output => {
            unsafe {
                reg_write(
                    GPIO_FUNC_OUT_SEL_BASE + (pin as usize) * 4,
                    SIG_GPIO_OUT_IDX,
                )
            };

            set_enable(pin, true);
            mark_output(pin, true);
        }
        PinMode::Input => {
            set_enable(pin, false);
            mark_output(pin, false);
        }
    }
    Ok(())
}

pub fn write(pin: u8, high: bool) -> KResult<()> {
    check_pin(pin)?;
    if !is_output(pin) {
        return Err(KError::PermissionDenied);
    }
    set_level(pin, high);
    Ok(())
}

pub fn read(pin: u8) -> KResult<bool> {
    check_pin(pin)?;
    let level = if pin < 32 {
        (unsafe { reg_read(GPIO_IN_REG) } & (1u32 << pin)) != 0
    } else {
        (unsafe { reg_read(GPIO_IN1_REG) } & (1u32 << (pin - 32))) != 0
    };
    Ok(level)
}

pub fn toggle(pin: u8) -> KResult<()> {
    check_pin(pin)?;
    if !is_output(pin) {
        return Err(KError::PermissionDenied);
    }
    let current = read_output_latch(pin);
    set_level(pin, !current);
    Ok(())
}
