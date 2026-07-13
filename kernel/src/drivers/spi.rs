#![allow(dead_code)]

//! Driver de bus SPI maestro (Fase 3).
//!
//! Envuelve el periférico `SPI2` de esp-hal detrás de un `Mutex` global. Los
//! periféricos se reciben desde `main` (NO se roban con `Peripherals::steal()`).
//!
//! Pines por defecto: SCK=GPIO12, MOSI=GPIO11, MISO=GPIO13. Sin chip-select por
//! hardware: el CS lo gestiona quien use el bus (p. ej. vía `drivers::gpio`).

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

/// Tamaño del buffer de descarte para transferencias unidireccionales.
const SCRATCH: usize = 64;

type SpiDriver = Spi<'static, Blocking>;

static SPI_BUS: Mutex<Option<SpiDriver>> = Mutex::new(None);

/// Inicializa el bus SPI con los periféricos entregados por `main`.
///
/// Genérico sobre los pines: SCK/MOSI son salidas, MISO es entrada.
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

/// ¿Está el bus inicializado?
pub fn is_ready() -> bool {
    SPI_BUS.lock().is_some()
}

/// Transferencia full-duplex: envía `tx` mientras recibe en `rx`
/// (`rx.len()` bytes se reciben; el hardware exige `rx.len() >= tx.len()`).
pub fn transfer(tx: &[u8], rx: &mut [u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();

    let spi = guard.as_mut().ok_or(KError::IoError)?;

    SpiBus::transfer(spi, rx, tx).map_err(|_| KError::IoError)
}

/// Envía `tx` descartando lo recibido.
pub fn write_bytes(tx: &[u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();
    let spi = guard.as_mut().ok_or(KError::IoError)?;
    SpiBus::write(spi, tx).map_err(|_| KError::IoError)
}

/// Lee `rx.len()` bytes reloj-generando ceros por MOSI.
pub fn read_bytes(rx: &mut [u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();
    let spi = guard.as_mut().ok_or(KError::IoError)?;
    SpiBus::read(spi, rx).map_err(|_| KError::IoError)
}

/// Nodo `/dev/spi0`: `write` envía; `read` recibe reloj-generando ceros.
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
}

/// Handle del dispositivo para registrarlo en devfs.
pub fn devfs_device() -> Arc<dyn Device> {
    Arc::new(Spi0Device)
}
