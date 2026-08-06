#![allow(dead_code)]

// /dev/passwd: sets the SSH login credential with the password salted+hashed before it
// ever touches flash (SP2 known-issue #3, see README §9). Hashing happens here, in the
// kernel, and nowhere else: userland has no crypto/RNG surface, so `passwd(1)` hands the
// raw password across the ioctl boundary (unavoidable -- it has to cross *somewhere*) and
// this driver is the only thing that reads it. It never writes the plaintext to
// `/etc/passwd`; only the salt+hash line does. See `ssh::auth::check_password` for the
// read side.

use crate::prelude::*;
use crate::vfs::file::OpenFlags;
use esp_hal::rng::Rng;
use rand_core::RngCore;
use sha2::{Digest, Sha256};

use super::ssh::crypto_rng::HwRng;

pub const SET_PASSWORD_CMD: u32 = 0;

const USER_MAX: usize = 64;
const PASS_MAX: usize = 128;
const SALT_LEN: usize = 16;

const O_WRONLY_CREATE_TRUNC: u32 = 0x0002 | 0x0100 | 0x0400;

/// Mirrors `passwd(1)`'s userland struct: `{user_ptr, user_len, pass_ptr, pass_len}`.
#[repr(C)]
struct PasswdReq {
    user_ptr: usize,
    user_len: usize,
    pass_ptr: usize,
    pass_len: usize,
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_push(byte: u8, out: &mut String) {
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0xf) as usize] as char);
}

/// Builds the `/etc/passwd` line for `user:pass` in the `$s5$<salt-hex>$<hash-hex>`
/// format (`s5` = "salted SHA-256", scheme tag so a future migration can recognize and
/// upgrade it). Rejects the same `:`/`\n` shapes `passwd(1)` already rejected client-side
/// -- re-checked here because the ioctl boundary is the actual trust boundary, not the
/// userland binary that happens to be the only caller today.
fn hash_line(user: &str, password: &[u8]) -> KResult<String> {
    if user.is_empty() || user.contains(':') || user.contains('\n') {
        return Err(KError::InvalidArgument);
    }
    if password.is_empty() || password.contains(&b':') || password.contains(&b'\n') {
        return Err(KError::InvalidArgument);
    }

    let mut salt = [0u8; SALT_LEN];
    let rng_periph = unsafe { esp_hal::peripherals::RNG::steal() };
    HwRng::new(Rng::new(rng_periph)).fill_bytes(&mut salt);

    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password);
    let hash = hasher.finalize();

    let mut line = String::from(user);
    line.push(':');
    line.push_str("$s5$");
    for b in salt {
        hex_push(b, &mut line);
    }
    line.push('$');
    for b in hash {
        hex_push(b, &mut line);
    }
    line.push('\n');
    Ok(line)
}

struct PasswdDevice;

impl crate::vfs::devfs::Device for PasswdDevice {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> KResult<usize> {
        Err(KError::NotSupported)
    }
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            SET_PASSWORD_CMD => {
                crate::syscall::handler::validate_user(arg, core::mem::size_of::<PasswdReq>())?;
                let req = unsafe { &*(arg as *const PasswdReq) };
                if req.user_len == 0
                    || req.user_len > USER_MAX
                    || req.pass_len == 0
                    || req.pass_len > PASS_MAX
                {
                    return Err(KError::InvalidArgument);
                }
                crate::syscall::handler::validate_user(req.user_ptr, req.user_len)?;
                crate::syscall::handler::validate_user(req.pass_ptr, req.pass_len)?;

                let mut ubuf = [0u8; USER_MAX];
                let mut pbuf = [0u8; PASS_MAX];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        req.user_ptr as *const u8,
                        ubuf.as_mut_ptr(),
                        req.user_len,
                    );
                    core::ptr::copy_nonoverlapping(
                        req.pass_ptr as *const u8,
                        pbuf.as_mut_ptr(),
                        req.pass_len,
                    );
                }
                let user = core::str::from_utf8(&ubuf[..req.user_len])
                    .map_err(|_| KError::InvalidArgument)?;
                let pass = &pbuf[..req.pass_len];

                let line = hash_line(user, pass)?;

                let fd = crate::vfs::open("/etc/passwd", OpenFlags(O_WRONLY_CREATE_TRUNC))?;
                let res = crate::vfs::write(fd, line.as_bytes());
                let _ = crate::vfs::close(fd);
                res
            }
            _ => Err(KError::InvalidArgument),
        }
    }
}

pub fn devfs_device() -> Arc<dyn crate::vfs::devfs::Device> {
    Arc::new(PasswdDevice)
}
