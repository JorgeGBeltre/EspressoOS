//! Driver SPI maestro (borrador — Fase 3).
//!
//! Bus SPI2 en modo maestro full-duplex para periféricos externos (pantallas,
//! sensores, flash auxiliar). Envuelve `esp_hal::spi::master::Spi` y guarda el
//! objeto del HAL en un `static Mutex<Option<...>>` propio del módulo, de modo
//! que las funciones libres (`init`, `transfer`) respeten las firmas simples del
//! contrato (§3.9) sin tener que pasar el periférico en cada llamada.
//!
//! Todas las funciones devuelven `KResult` (§3.9); los errores de esp-hal se
//! convierten a `KError` en la frontera del driver (nunca se propaga
//! `Result<_, ()>` hacia arriba).
// COMPILE-STATUS: borrador (sin compilar)
#![allow(dead_code)]

use crate::arch::xtensa::sync::Mutex;
use crate::prelude::*;

use esp_hal::spi::master::{Config, Spi};
use esp_hal::spi::Mode;
use esp_hal::Blocking;

/// Frecuencia de reloj SCK por defecto (10 MHz). Ajustar por placa/periférico.
const SPI_FREQ_HZ: u32 = 10_000_000;

// Pines por defecto del bus SPI2 (según el cheatsheet del contrato §1.6).
// NOTA: son informativos. esp-hal 0.23 vincula el pin por su *singleton*
// (`peripherals.GPIO12`), no por número; ver `init()`. Si la placa rutea el bus
// a otros pines, cambiar los singletons usados en `init()` (y estos comentarios).
const PIN_SCK: u8 = 12; // GPIO12 -> SCK
const PIN_MOSI: u8 = 11; // GPIO11 -> MOSI
const PIN_MISO: u8 = 13; // GPIO13 -> MISO

/// Tipo concreto del driver del HAL almacenado en el estado del módulo.
///
/// SUPUESTO DE API (?): en esp-hal 0.23 el driver bloqueante es
/// `Spi<'d, Blocking>`. Como el objeto se construye a partir de los singletons
/// de periférico/pines (con vida `'static`), la vida es `'static`. Si el
/// marcador de modo tuviera otro nombre/forma en la versión instalada, este
/// alias es el ÚNICO punto a tocar.
type SpiDriver = Spi<'static, Blocking>;

/// Estado del bus: `None` hasta que `init()` lo construye. Protegido por `Mutex`
/// (SMP-safe, §3.2.4). Nunca `static mut`.
///
/// SUPUESTO (?): `Mutex<T>: Sync` requiere `T: Send`. Se asume que el driver
/// `Spi` de esp-hal es `Send` (los drivers de periférico del HAL lo son). Si no
/// lo fuera, habría que envolverlo o cambiar la primitiva de sincronización.
static SPI_BUS: Mutex<Option<SpiDriver>> = Mutex::new(None);

/// Inicializa el bus SPI maestro (SPI2 + pines de placa). [CANÓNICO §3.9]
///
/// Debe llamarse UNA sola vez, en la secuencia de arranque (§5, paso 13).
///
/// SUPUESTO RIESGOSO (?): la firma pública no recibe periféricos, así que aquí
/// se obtienen con `Peripherals::steal()` (contrato §5: «la variante con
/// periférico es un detalle interno del init»). Implicaciones:
///  - Solo puede invocarse una vez; una segunda llamada volvería a robar y
///    duplicaría la propiedad de SPI2/pines.
///  - SPI2 + GPIO11/12/13 NO deben solaparse con periféricos que use otro
///    módulo (`main` solo usa GPIO2 para el LED, así que no hay conflicto).
///  - El agente de integración puede sustituir este `steal()` por un paso
///    explícito de `peripherals.SPI2`/pines si prefiere un reparto limpio.
pub fn init() -> KResult<()> {
    // SAFETY: ver nota de la doc. Robo único de los singletons que este driver
    // posee en exclusiva durante toda la vida del kernel.
    let p = unsafe { esp_hal::peripherals::Peripherals::steal() };

    // Configuración del bus: modo 0 (CPOL=0, CPHA=0), frecuencia por defecto.
    // SUPUESTO DE API (?): en 0.23 `Config` es no-exhaustivo (construir con
    // `default()` + `with_*`); `with_frequency` toma `fugit::HertzU32` y
    // `with_mode` toma `esp_hal::spi::Mode::_0..=_3`.
    let config = Config::default()
        .with_frequency(fugit::HertzU32::Hz(SPI_FREQ_HZ))
        .with_mode(Mode::_0);

    // Construcción del maestro + asignación de pines. `Spi::new` devuelve
    // `Result` en 0.23; su error se convierte a `KError::IoError`.
    // SUPUESTO DE API (?): `with_sck/with_mosi/with_miso` consumen y devuelven
    // `Self` (no `Result`), encadenables tras `?`.
    let spi = Spi::new(p.SPI2, config)
        .map_err(|_| KError::IoError)?
        .with_sck(p.GPIO12)
        .with_mosi(p.GPIO11)
        .with_miso(p.GPIO13);

    // Publicar el driver ya inicializado en el estado del módulo.
    let mut guard = SPI_BUS.lock();
    *guard = Some(spi);
    Ok(())
}

/// Transferencia full-duplex: escribe `tx` y llena `rx` simultáneamente. [CANÓNICO §3.9]
///
/// Para una transacción full-duplex real ambos búferes deberían tener la misma
/// longitud; si difieren, el HAL rellena/trunca según su implementación. Si el
/// bus no se ha inicializado se devuelve `KError::IoError`.
pub fn transfer(tx: &[u8], rx: &mut [u8]) -> KResult<()> {
    let mut guard = SPI_BUS.lock();
    // `as_mut()` sobre el `Option`: `None` => bus sin inicializar.
    let spi = guard.as_mut().ok_or(KError::IoError)?;

    // SUPUESTO DE API (?): el driver bloqueante de 0.23 expone el método
    // inherente `transfer(read: &mut [u8], write: &[u8])` (mismo orden de
    // argumentos que el trait `embedded_hal::spi::SpiBus`: primero lectura,
    // luego escritura). Por eso se llama `transfer(rx, tx)`.
    //
    // ALTERNATIVA si NO fuese inherente: importar `embedded_hal::spi::SpiBus`
    // (crate `embedded-hal`, ver `needs_crates`) y llamar exactamente igual, ya
    // que esp-hal implementa ese trait para el `Spi` bloqueante.
    spi.transfer(rx, tx).map_err(|_| KError::IoError)
}
