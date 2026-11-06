#![allow(dead_code)]

use crate::arch::xtensa::sync::SpinLock;
use crate::prelude::*;
use core::cell::UnsafeCell;

const KLOG_CAP: usize = 4096;

struct KLogRing {
    buf: [u8; KLOG_CAP],
    head: usize,
    len: usize,
}

impl KLogRing {
    const fn new() -> Self {
        Self {
            buf: [0; KLOG_CAP],
            head: 0,
            len: 0,
        }
    }

    fn drop_oldest_line(&mut self) {
        while self.len > 0 {
            let b = self.buf[self.head];
            self.head = (self.head + 1) % KLOG_CAP;
            self.len -= 1;
            if b == b'\n' {
                break;
            }
        }
    }

    fn push_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let slice = if data.len() > KLOG_CAP {
            &data[data.len() - KLOG_CAP..]
        } else {
            data
        };
        while self.len + slice.len() > KLOG_CAP {
            self.drop_oldest_line();
            if self.len + slice.len() > KLOG_CAP && self.len > 0 {
                self.head = (self.head + 1) % KLOG_CAP;
                self.len -= 1;
            }
        }
        for &b in slice {
            let idx = (self.head + self.len) % KLOG_CAP;
            self.buf[idx] = b;
            self.len += 1;
        }
    }
}

struct KLogCell {
    lock: SpinLock,
    ring: UnsafeCell<KLogRing>,
}

unsafe impl Sync for KLogCell {}

static KLOG: KLogCell = KLogCell {
    lock: SpinLock::new(),
    ring: UnsafeCell::new(KLogRing::new()),
};

fn fmt_u64_dec(mut n: u64, out: &mut [u8]) -> usize {
    if n == 0 {
        if !out.is_empty() {
            out[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut tmp = [0u8; 20];
    let mut i = 0;
    while n > 0 && i < tmp.len() {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let count = core::cmp::min(i, out.len());
    for k in 0..count {
        out[k] = tmp[i - 1 - k];
    }
    count
}

/// Appends a timestamped diagnostic message `[ssss.mmm] <msg>\n` to the kernel ring buffer.
/// Uses fixed stack buffers and never allocates on the heap while holding the spinlock.
pub fn log_msg(msg: &str) {
    let trimmed = msg.trim_matches(|c| c == '\r' || c == '\n');
    if trimmed.is_empty() {
        return;
    }

    let ms = crate::arch::xtensa::timer::uptime_ms();
    let secs = ms / 1000;
    let rem = ms % 1000;

    let mut hdr = [b' '; 24];
    let mut p = 0usize;
    hdr[p] = b'[';
    p += 1;
    let mut dec = [0u8; 20];
    let slen = fmt_u64_dec(secs, &mut dec);
    let pad = 5usize.saturating_sub(slen);
    for _ in 0..pad {
        hdr[p] = b' ';
        p += 1;
    }
    hdr[p..p + slen].copy_from_slice(&dec[..slen]);
    p += slen;
    hdr[p] = b'.';
    p += 1;
    hdr[p] = b'0' + ((rem / 100) % 10) as u8;
    hdr[p + 1] = b'0' + ((rem / 10) % 10) as u8;
    hdr[p + 2] = b'0' + (rem % 10) as u8;
    hdr[p + 3] = b']';
    hdr[p + 4] = b' ';
    p += 5;

    crate::arch::xtensa::interrupts::critical_section(|| {
        KLOG.lock.lock();
        let ring = unsafe { &mut *KLOG.ring.get() };
        ring.push_bytes(&hdr[..p]);
        ring.push_bytes(trimmed.as_bytes());
        ring.push_bytes(b"\n");
        KLOG.lock.unlock();
    });
}

/// Captures a UTF-8 snapshot of the current kernel ring buffer for `/proc/dmesg`.
pub fn snapshot() -> String {
    let mut raw = [0u8; KLOG_CAP];
    let n = crate::arch::xtensa::interrupts::critical_section(|| {
        KLOG.lock.lock();
        let ring = unsafe { &*KLOG.ring.get() };
        let count = ring.len;
        for i in 0..count {
            raw[i] = ring.buf[(ring.head + i) % KLOG_CAP];
        }
        KLOG.lock.unlock();
        count
    });
    String::from_utf8_lossy(&raw[..n]).into_owned()
}

