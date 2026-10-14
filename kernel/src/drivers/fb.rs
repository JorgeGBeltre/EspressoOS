#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::devfs::Device;

pub const FB_WIDTH: usize = 128;
pub const FB_HEIGHT: usize = 64;
pub const FB_SIZE: usize = (FB_WIDTH * FB_HEIGHT) / 8; // 1024 bytes

pub const FBIO_GET_VSCREENINFO: u32 = 0x4600;
pub const FBIO_CLEAR: u32 = 0x4601;
pub const FBIO_FILL: u32 = 0x4602;
pub const FBIO_INVERT: u32 = 0x4603;

static FRAMEBUFFER: Mutex<[u8; FB_SIZE]> = Mutex::new([0u8; FB_SIZE]);

pub struct FramebufferDevice;

impl Device for FramebufferDevice {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        let fb = FRAMEBUFFER.lock();
        let start = off as usize;
        if start >= FB_SIZE {
            return Ok(0);
        }
        let count = core::cmp::min(FB_SIZE - start, buf.len());
        buf[..count].copy_from_slice(&fb[start..start + count]);
        Ok(count)
    }

    fn write(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        let mut fb = FRAMEBUFFER.lock();
        let start = off as usize;
        if start >= FB_SIZE {
            return Ok(0);
        }
        let count = core::cmp::min(FB_SIZE - start, buf.len());
        fb[start..start + count].copy_from_slice(&buf[..count]);
        Ok(count)
    }

    fn ioctl(&self, cmd: u32, _arg: usize) -> KResult<usize> {
        match cmd {
            FBIO_GET_VSCREENINFO => {
                // Return (width << 16) | height
                Ok(((FB_WIDTH as usize) << 16) | (FB_HEIGHT as usize))
            }
            FBIO_CLEAR => {
                let mut fb = FRAMEBUFFER.lock();
                for b in fb.iter_mut() {
                    *b = 0;
                }
                Ok(0)
            }
            FBIO_FILL => {
                let mut fb = FRAMEBUFFER.lock();
                for b in fb.iter_mut() {
                    *b = 0xFF;
                }
                Ok(0)
            }
            FBIO_INVERT => {
                let mut fb = FRAMEBUFFER.lock();
                for b in fb.iter_mut() {
                    *b = !*b;
                }
                Ok(0)
            }
            _ => Err(KError::NotSupported),
        }
    }
}

pub fn devfs_device() -> Arc<dyn Device> {
    Arc::new(FramebufferDevice)
}
