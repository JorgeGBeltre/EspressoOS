#![allow(dead_code)]

use crate::prelude::*;
use esp_hal::sha::{Sha, Sha256};
use nb::block;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let sha_periph = unsafe { esp_hal::peripherals::SHA::steal() };
    let mut sha = Sha::new(sha_periph);
    let mut hasher = sha.start::<Sha256>();

    let mut remaining = data;
    while !remaining.is_empty() {
        remaining = block!(hasher.update(remaining)).unwrap();
    }

    let mut output = [0u8; 32];
    block!(hasher.finish(&mut output)).unwrap();
    output
}

// ---- /dev/sha0: SHA-256 por ioctl (SP2 R5). Molde D-1 (struct tipado + validación). ----

pub const SHA256_CMD: u32 = 0;
const SHA_IN_MAX: usize = 4096; // D-2: cota del input (hasta 4KB).

/// Struct D-1: `{in_ptr, in_len, out_ptr}`; `out` recibe 32 bytes (el hash). D-3: el
/// resultado (datos) viaja por el struct del ioctl, no es estado del driver.
#[repr(C)]
struct ShaReq {
    in_ptr: usize,
    in_len: usize,
    out_ptr: usize,
}

struct ShaDevice;

impl crate::vfs::devfs::Device for ShaDevice {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            SHA256_CMD => {
                crate::syscall::handler::validate_user(arg, core::mem::size_of::<ShaReq>())?;
                let req = unsafe { &*(arg as *const ShaReq) };
                if req.in_len > SHA_IN_MAX {
                    return Err(KError::InvalidArgument);
                }
                crate::syscall::handler::validate_user(req.in_ptr, req.in_len)?;
                crate::syscall::handler::validate_user(req.out_ptr, 32)?;
                let mut kin = [0u8; SHA_IN_MAX];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        req.in_ptr as *const u8,
                        kin.as_mut_ptr(),
                        req.in_len,
                    );
                }
                let hash = sha256(&kin[..req.in_len]);
                unsafe {
                    core::ptr::copy_nonoverlapping(hash.as_ptr(), req.out_ptr as *mut u8, 32);
                }
                Ok(32)
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(ShaDevice)
}
