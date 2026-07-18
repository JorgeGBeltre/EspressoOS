#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::devfs::Device;

use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::I2C0;
use esp_hal::Blocking;

pub const SCAN_FIRST: u8 = 0x08;

pub const SCAN_LAST: u8 = 0x77;

type I2cDriver = I2c<'static, Blocking>;

static I2C_BUS: Mutex<Option<I2cDriver>> = Mutex::new(None);

pub fn init<SDA, SCL>(i2c0: I2C0, sda: SDA, scl: SCL) -> KResult<()>
where
    SDA: Peripheral + 'static,
    SDA::P: PeripheralOutput,
    SCL: Peripheral + 'static,
    SCL::P: PeripheralOutput,
{
    let i2c = I2c::new(i2c0, Config::default())
        .map_err(|_| KError::IoError)?
        .with_sda(sda)
        .with_scl(scl);

    let mut guard = I2C_BUS.lock();
    *guard = Some(i2c);
    Ok(())
}

pub fn is_ready() -> bool {
    I2C_BUS.lock().is_some()
}

pub fn read(addr: u8, buf: &mut [u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;

    i2c.read(addr, buf).map_err(|_| KError::IoError)
}

pub fn write(addr: u8, buf: &[u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;

    i2c.write(addr, buf).map_err(|_| KError::IoError)
}

pub fn write_read(addr: u8, wr: &[u8], rd: &mut [u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;

    i2c.write_read(addr, wr, rd).map_err(|_| KError::IoError)
}

pub fn probe(addr: u8) -> bool {
    let mut guard = I2C_BUS.lock();
    match guard.as_mut() {
        Some(i2c) => i2c.write(addr, &[]).is_ok(),
        None => false,
    }
}

pub const I2C_PROBE: u32 = 0;
pub const I2C_READ: u32 = 1;
pub const I2C_WRITE: u32 = 2;
const I2C_MAX: usize = 64; // D-2: transferencias 1..64 (como documenta el builtin).

/// Struct D-1 para `i2c read`/`i2c write`: `{addr, buf_ptr, len}`. Todos `usize`.
#[repr(C)]
struct I2cReq {
    addr: usize,
    buf_ptr: usize,
    len: usize,
}

/// Camino común read/write con validación D-1 (struct + puntero interno) y límite D-2.
/// `is_write=true` copia user→bus; false lee bus→user (D-3: los bytes del bus viajan por el
/// struct del ioctl porque son datos, no estado del driver).
fn i2c_xfer(arg: usize, is_write: bool) -> KResult<usize> {
    crate::syscall::handler::validate_user(arg, core::mem::size_of::<I2cReq>())?;
    let req = unsafe { &*(arg as *const I2cReq) };
    if req.len == 0 || req.len > I2C_MAX {
        return Err(KError::InvalidArgument);
    }
    crate::syscall::handler::validate_user(req.buf_ptr, req.len)?;
    let addr = req.addr as u8;
    let mut kbuf = [0u8; I2C_MAX];
    if is_write {
        unsafe {
            core::ptr::copy_nonoverlapping(req.buf_ptr as *const u8, kbuf.as_mut_ptr(), req.len);
        }
        write(addr, &kbuf[..req.len])?;
    } else {
        read(addr, &mut kbuf[..req.len])?;
        unsafe {
            core::ptr::copy_nonoverlapping(kbuf.as_ptr(), req.buf_ptr as *mut u8, req.len);
        }
    }
    Ok(req.len)
}

pub struct I2c0Device;

impl Device for I2c0Device {
    fn read(&self, off: u64, buf: &mut [u8]) -> KResult<usize> {
        read(off as u8, buf)?;
        Ok(buf.len())
    }
    fn write(&self, off: u64, buf: &[u8]) -> KResult<usize> {
        write(off as u8, buf)?;
        Ok(buf.len())
    }
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            I2C_PROBE => Ok(probe(arg as u8) as usize),
            I2C_READ => i2c_xfer(arg, false),
            I2C_WRITE => i2c_xfer(arg, true),
            _ => Err(KError::NotSupported),
        }
    }
}

pub fn devfs_device() -> Arc<dyn Device> {
    Arc::new(I2c0Device)
}
