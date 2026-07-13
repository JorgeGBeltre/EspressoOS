#![allow(dead_code)]

//! Driver de bus I2C maestro (Fase 3).
//!
//! Envuelve el periférico `I2C0` de esp-hal detrás de un `Mutex` global. Los
//! periféricos se reciben desde `main` (NO se roban con `Peripherals::steal()`,
//! que sería incorrecto tras `esp_hal::init`).
//!
//! Pines por defecto en la ESP32-S3-WROOM-1: SDA=GPIO8, SCL=GPIO9. No colisionan
//! con el flash/PSRAM octal (GPIO26-37) ni con el USB nativo (GPIO19/20).

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;
use crate::vfs::devfs::Device;

use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::I2C0;
use esp_hal::Blocking;

/// Primera dirección de 7 bits válida para escaneo (0x00-0x07 reservadas).
pub const SCAN_FIRST: u8 = 0x08;
/// Última dirección de 7 bits válida para escaneo (0x78-0x7F reservadas).
pub const SCAN_LAST: u8 = 0x77;

type I2cDriver = I2c<'static, Blocking>;

static I2C_BUS: Mutex<Option<I2cDriver>> = Mutex::new(None);

/// Inicializa el bus I2C con los periféricos entregados por `main`.
///
/// Genérico sobre los pines para no fijar sus tipos concretos (SDA/SCL son
/// salidas open-drain: ambos `PeripheralOutput`).
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

/// ¿Está el bus inicializado?
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

/// Sondea una dirección con una transacción de dirección-solo (escritura de 0
/// bytes). Devuelve `true` si el dispositivo hace ACK.
pub fn probe(addr: u8) -> bool {
    let mut guard = I2C_BUS.lock();
    match guard.as_mut() {
        Some(i2c) => i2c.write(addr, &[]).is_ok(),
        None => false,
    }
}

/// Nodo `/dev/i2c0`: `off` codifica la dirección de 7 bits del esclavo.
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
        // cmd 0 = probe(arg as addr) -> 1 si presente, 0 si no.
        match cmd {
            0 => Ok(probe(arg as u8) as usize),
            _ => Err(KError::NotSupported),
        }
    }
}

/// Handle del dispositivo para registrarlo en devfs.
pub fn devfs_device() -> Arc<dyn Device> {
    Arc::new(I2c0Device)
}
