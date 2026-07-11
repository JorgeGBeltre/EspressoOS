//! Driver I2C maestro (borrador — Fase 3).
//!
//! Bus I2C0 en modo maestro para sensores/EEPROM. Se usa como periférico de
//! prueba en el criterio de aceptación de la Fase 3. Envuelve
//! `esp_hal::i2c::master::I2c` y guarda el objeto del HAL en un
//! `static Mutex<Option<...>>` del módulo, para respetar las firmas simples del
//! contrato (§3.9) sin pasar el periférico en cada llamada.
//!
//! Todas las funciones devuelven `KResult` (§3.9); los errores de esp-hal se
//! convierten a `KError` en la frontera del driver (nunca se propaga
//! `Result<_, ()>` hacia arriba).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

use esp_hal::i2c::master::{Config, I2c};
use esp_hal::Blocking;

// Pines por defecto del bus I2C0 (según el cheatsheet del contrato §1.7).
// NOTA: informativos. esp-hal 0.23 vincula el pin por su *singleton*
// (`peripherals.GPIO8`), no por número; ver `init()`. Ajustar los singletons de
// `init()` si la placa rutea el bus a otros pines.
const PIN_SDA: u8 = 8; // GPIO8 -> SDA
const PIN_SCL: u8 = 9; // GPIO9 -> SCL

/// Tipo concreto del driver del HAL almacenado en el estado del módulo.
///
/// SUPUESTO DE API (?): en esp-hal 0.23 el driver bloqueante es
/// `I2c<'d, Blocking>`. Al construirse a partir de singletons `'static`, la vida
/// es `'static`. Si el marcador de modo difiere en la versión instalada, este
/// alias es el ÚNICO punto a tocar.
type I2cDriver = I2c<'static, Blocking>;

/// Estado del bus: `None` hasta que `init()` lo construye. Protegido por `Mutex`
/// (SMP-safe, §3.2.4). Nunca `static mut`.
///
/// SUPUESTO (?): `Mutex<T>: Sync` requiere `T: Send`; se asume que el driver
/// `I2c` de esp-hal es `Send` (los drivers de periférico del HAL lo son).
static I2C_BUS: Mutex<Option<I2cDriver>> = Mutex::new(None);

/// Inicializa el bus I2C maestro (I2C0 + pines de placa). [CANÓNICO §3.9]
///
/// Debe llamarse UNA sola vez, en la secuencia de arranque (§5, paso 13).
///
/// SUPUESTO RIESGOSO (?): la firma pública no recibe periféricos, así que aquí
/// se obtienen con `Peripherals::steal()` (contrato §5: «la variante con
/// periférico es un detalle interno del init»). Implicaciones:
///  - Solo puede invocarse una vez.
///  - I2C0 + GPIO8/9 NO deben solaparse con periféricos que use otro módulo
///    (`main` solo usa GPIO2, así que no hay conflicto).
///  - El agente de integración puede sustituir el `steal()` por un paso
///    explícito de `peripherals.I2C0`/pines si prefiere un reparto limpio.
pub fn init() -> KResult<()> {
    // SAFETY: ver nota de la doc. Robo único de los singletons que este driver
    // posee en exclusiva durante toda la vida del kernel.
    let p = unsafe { esp_hal::peripherals::Peripherals::steal() };

    // SUPUESTO DE API (?): `I2c::new(periférico, Config)` devuelve `Result` en
    // 0.23; `Config::default()` fija ~100 kHz. `with_sda/with_scl` consumen y
    // devuelven `Self`. Para otra frecuencia: `Config::default().with_frequency(
    // fugit::HertzU32::kHz(400))` (Fast-mode), análogo a SPI.
    let i2c = I2c::new(p.I2C0, Config::default())
        .map_err(|_| KError::IoError)?
        .with_sda(p.GPIO8)
        .with_scl(p.GPIO9);

    let mut guard = I2C_BUS.lock();
    *guard = Some(i2c);
    Ok(())
}

/// Lee `buf.len()` bytes desde el dispositivo `addr` (dirección de 7 bits). [CANÓNICO §3.9]
///
/// Devuelve `KError::IoError` si el bus no está inicializado o falla la
/// transacción (NACK, arbitraje, timeout del HAL).
pub fn read(addr: u8, buf: &mut [u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;
    // SUPUESTO DE API (?): método inherente `read(addr, buf) -> Result<(),
    // esp_hal::i2c::master::Error>` en 0.23. `addr: u8` (7 bits) es aceptado
    // directamente o vía `Into<I2cAddress>`.
    i2c.read(addr, buf).map_err(|_| KError::IoError)
}

/// Escribe `buf` al dispositivo `addr` (dirección de 7 bits). [CANÓNICO §3.9]
pub fn write(addr: u8, buf: &[u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;
    // SUPUESTO DE API (?): método inherente `write(addr, buf)` en 0.23.
    i2c.write(addr, buf).map_err(|_| KError::IoError)
}

/// Escribe `wr` y, sin liberar el bus (repeated-start), lee en `rd`. [CANÓNICO §3.9]
///
/// Patrón típico «puntero de registro + lectura» de sensores/EEPROM.
pub fn write_read(addr: u8, wr: &[u8], rd: &mut [u8]) -> KResult<()> {
    let mut guard = I2C_BUS.lock();
    let i2c = guard.as_mut().ok_or(KError::IoError)?;
    // SUPUESTO DE API (?): método inherente `write_read(addr, wr, rd)` en 0.23
    // (orden: dirección, buffer a escribir, buffer a leer).
    i2c.write_read(addr, wr, rd).map_err(|_| KError::IoError)
}
