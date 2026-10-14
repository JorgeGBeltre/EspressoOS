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

pub const GPIO_IOC_CONFIGURE: u32 = 0x4701;
pub const GPIO_IOC_WRITE: u32 = 0x4702;
pub const GPIO_IOC_READ: u32 = 0x4703;
pub const GPIO_IOC_TOGGLE: u32 = 0x4704;

pub struct GpioDevice;

impl crate::vfs::devfs::Device for GpioDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let lo = OUTPUT_MASK_LO.load(Ordering::Relaxed);
        let hi = OUTPUT_MASK_HI.load(Ordering::Relaxed);
        let s = alloc::format!("gpio: out_lo={:#010x} out_hi={:#010x}\n", lo, hi);
        let bytes = s.as_bytes();
        let off = off as usize;
        if off >= bytes.len() {
            return Ok(0);
        }
        let avail = &bytes[off..];
        let n = buf.len().min(avail.len());
        buf[..n].copy_from_slice(&avail[..n]);
        Ok(n)
    }

    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        if let Ok(cmd_str) = core::str::from_utf8(buf) {
            let trimmed = cmd_str.trim();
            let mut parts = trimmed.split_whitespace();
            if let Some(verb) = parts.next() {
                if let Some(pin_str) = parts.next() {
                    if let Ok(pin) = pin_str.parse::<u8>() {
                        match verb {
                            "mode" => {
                                if let Some(m) = parts.next() {
                                    let mode = if m == "out" || m == "output" {
                                        PinMode::Output
                                    } else {
                                        PinMode::Input
                                    };
                                    configure(pin, mode)?;
                                    return Ok(buf.len());
                                }
                            }
                            "write" => {
                                if let Some(v) = parts.next() {
                                    let high = v == "1" || v == "high" || v == "true";
                                    write(pin, high)?;
                                    return Ok(buf.len());
                                }
                            }
                            "toggle" => {
                                toggle(pin)?;
                                return Ok(buf.len());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(buf.len())
    }

    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        let pin = (arg & 0xFF) as u8;
        match cmd {
            GPIO_IOC_CONFIGURE => {
                let mode = if ((arg >> 8) & 1) != 0 {
                    PinMode::Output
                } else {
                    PinMode::Input
                };
                configure(pin, mode)?;
                Ok(0)
            }
            GPIO_IOC_WRITE => {
                let high = ((arg >> 8) & 1) != 0;
                write(pin, high)?;
                Ok(0)
            }
            GPIO_IOC_READ => {
                let val = read(pin)?;
                Ok(if val { 1 } else { 0 })
            }
            GPIO_IOC_TOGGLE => {
                toggle(pin)?;
                Ok(0)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(GpioDevice)
}
