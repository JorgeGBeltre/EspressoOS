#![allow(dead_code)]

use crate::arch::xtensa::sync::SpinLock;
use crate::prelude::*;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

const USB_SERIAL_JTAG_BASE: usize = 0x6003_8000;

const EP1_REG: usize = USB_SERIAL_JTAG_BASE + 0x0000;

const EP1_CONF_REG: usize = USB_SERIAL_JTAG_BASE + 0x0004;

const CONF_WR_DONE: u32 = 1 << 0;

const CONF_IN_EP_DATA_FREE: u32 = 1 << 1;

const CONF_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

const MAX_SPIN: u32 = 100_000;

const TX_BUF_LEN: usize = 512;

#[inline(always)]
unsafe fn reg_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn reg_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val)
}

struct Ring {
    buf: [u8; TX_BUF_LEN],
    head: usize,
    tail: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [0; TX_BUF_LEN],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    fn push(&mut self, b: u8) -> bool {
        if self.len >= TX_BUF_LEN {
            return false;
        }

        if let Some(slot) = self.buf.get_mut(self.tail) {
            *slot = b;
        }
        self.tail = (self.tail + 1) % TX_BUF_LEN;
        self.len += 1;
        true
    }

    fn peek(&self) -> Option<u8> {
        if self.len == 0 {
            None
        } else {
            self.buf.get(self.head).copied()
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.buf.get(self.head).copied();
        self.head = (self.head + 1) % TX_BUF_LEN;
        self.len -= 1;
        b
    }
}

struct ConsoleInner {
    tx: Ring,
}

impl ConsoleInner {
    const fn new() -> Self {
        Self { tx: Ring::new() }
    }

    fn drain_tx(&mut self) {
        let mut wrote_any = false;
        let mut guard: u32 = 0;

        while let Some(b) = self.tx.peek() {
            let free = unsafe { reg_read(EP1_CONF_REG) } & CONF_IN_EP_DATA_FREE != 0;
            if !free {
                guard = guard.wrapping_add(1);
                if guard > MAX_SPIN {
                    break;
                }
                core::hint::spin_loop();
                continue;
            }
            guard = 0;

            unsafe { reg_write(EP1_REG, b as u32) };
            let _ = self.tx.pop();
            wrote_any = true;
        }

        if wrote_any {
            unsafe { reg_write(EP1_CONF_REG, CONF_WR_DONE) };
        }
    }

    fn read_hw_byte(&mut self) -> Option<u8> {
        let avail = unsafe { reg_read(EP1_CONF_REG) } & CONF_OUT_EP_DATA_AVAIL != 0;
        if !avail {
            return None;
        }

        Some((unsafe { reg_read(EP1_REG) } & 0xFF) as u8)
    }
}

struct Console {
    lock: SpinLock,
    inner: UnsafeCell<ConsoleInner>,
}

unsafe impl Sync for Console {}

impl Console {
    const fn new() -> Self {
        Self {
            lock: SpinLock::new(),
            inner: UnsafeCell::new(ConsoleInner::new()),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut ConsoleInner) -> R) -> R {
        self.lock.lock();

        let r = f(unsafe { &mut *self.inner.get() });
        self.lock.unlock();
        r
    }
}

static CONSOLE: Console = Console::new();

static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init() -> KResult<()> {
    INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

pub fn write(buf: &[u8]) -> usize {
    match core::str::from_utf8(buf) {
        Ok(s) => esp_println::print!("{}", s),
        Err(_) => {
            for &b in buf {
                esp_println::print!("{}", b as char);
            }
        }
    }
    buf.len()
}

#[inline]
fn uart0_read_byte() -> Option<u8> {
    let uart = unsafe { &*esp_hal::peripherals::UART0::PTR };
    if uart.status().read().rxfifo_cnt().bits() > 0 {
        Some(uart.fifo().read().rxfifo_rd_byte().bits())
    } else {
        None
    }
}

pub fn read(buf: &mut [u8]) -> usize {
    let mut n = 0usize;
    while n < buf.len() {
        match uart0_read_byte() {
            Some(b) => {
                if let Some(slot) = buf.get_mut(n) {
                    *slot = b;
                }
                n += 1;
            }
            None => break,
        }
    }
    n
}

pub fn getc() -> Option<u8> {
    uart0_read_byte()
}
