#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::devfs::Device;

use esp_hal::gpio::interconnect::{PeripheralInput, PeripheralOutput};
use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::SPI2;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::spi::Mode;
use esp_hal::Blocking;

use embedded_hal::spi::SpiBus;

const SPI_FREQ_HZ: u32 = 10_000_000;

const SCRATCH: usize = 64;

type SpiDriver = Spi<'static, Blocking>;

static SPI_BUS: Mutex<Option<SpiDriver>> = Mutex::new(None);

pub fn init<SCK, MOSI, MISO>(spi2: SPI2, sck: SCK, mosi: MOSI, miso: MISO) -> KResult<()>
where
    SCK: Peripheral + 'static,
    SCK::P: PeripheralOutput,
    MOSI: Peripheral + 'static,
    MOSI::P: PeripheralOutput,
    MISO: Peripheral + 'static,
    MISO::P: PeripheralInput,
{
    let config = Config::default()
        .with_frequency(fugit::HertzU32::Hz(SPI_FREQ_HZ))
        .with_mode(Mode::_0);

    let spi = Spi::new(spi2, config)
        .map_err(|_| KError::IoError)?
        .with_sck(sck)
        .with_mosi(mosi)
        .with_miso(miso);

    let mut guard = SPI_BUS.lock();
    *guard = Some(spi);
    Ok(())
}

pub fn is_ready() -> bool {
    SPI_BUS.lock().is_some()
}

pub fn transfer(tx: &[u8], rx: &mut [u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();

    let spi = guard.as_mut().ok_or(KError::IoError)?;

    SpiBus::transfer(spi, rx, tx).map_err(|_| KError::IoError)
}

pub fn write_bytes(tx: &[u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();
    let spi = guard.as_mut().ok_or(KError::IoError)?;
    SpiBus::write(spi, tx).map_err(|_| KError::IoError)
}

pub fn read_bytes(rx: &mut [u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();
    let spi = guard.as_mut().ok_or(KError::IoError)?;
    SpiBus::read(spi, rx).map_err(|_| KError::IoError)
}

pub const SPI_TRANSFER: u32 = 0;
const SPI_MAX: usize = 64; // D-2.

/// Struct D-1 para `spi transfer`: `{buf_ptr, len}`. Full-duplex in-place: el buffer trae
/// los bytes TX a la entrada y recibe los RX a la salida (D-3: datos de bus por el struct).
#[repr(C)]
struct SpiReq {
    buf_ptr: usize,
    len: usize,
}

fn spi_transfer(arg: usize) -> KResult<usize> {
    crate::syscall::handler::validate_user(arg, core::mem::size_of::<SpiReq>())?;
    let req = unsafe { &*(arg as *const SpiReq) };
    if req.len == 0 || req.len > SPI_MAX {
        return Err(KError::InvalidArgument);
    }
    crate::syscall::handler::validate_user(req.buf_ptr, req.len)?;
    let mut tx = [0u8; SPI_MAX];
    let mut rx = [0u8; SPI_MAX];
    unsafe {
        core::ptr::copy_nonoverlapping(req.buf_ptr as *const u8, tx.as_mut_ptr(), req.len);
    }
    transfer(&tx[..req.len], &mut rx[..req.len])?;
    unsafe {
        core::ptr::copy_nonoverlapping(rx.as_ptr(), req.buf_ptr as *mut u8, req.len);
    }
    Ok(req.len)
}

pub struct Spi0Device;

impl Device for Spi0Device {
    fn read(&self, _off: u64, buf: &mut [u8]) -> KResult<usize> {
        read_bytes(buf)?;
        Ok(buf.len())
    }
    fn write(&self, _off: u64, buf: &[u8]) -> KResult<usize> {
        write_bytes(buf)?;
        Ok(buf.len())
    }
    fn ioctl(&self, cmd: u32, arg: usize) -> KResult<usize> {
        match cmd {
            SPI_TRANSFER => spi_transfer(arg),
            _ => Err(KError::NotSupported),
        }
    }
}

pub fn devfs_device() -> Arc<dyn Device> {
    Arc::new(Spi0Device)
}
